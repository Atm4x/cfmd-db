# PASS324 REPORT — Replication authority semantic compaction R&D

## Wall clock
- Start: 2026-09-29 05:46:05 UTC
- Functional freeze: 2026-09-29 06:06:24 UTC
- Useful boundary: 06:06:05 UTC
- Hard boundary: 06:10:05 UTC

## Goal
Determine whether P323's remaining replication-authority copy amplification can be removed by a canonical semantic snapshot without introducing a generic fallback, and establish an executable semantic boundary before changing the physical format.

## Delivered

### 1. Exact semantic-state projection
Added `replication/authority/semantic_snapshot.rs` as an executable test model of the complete replay-produced `ReplicationAuthorityJournal` semantic state. It intentionally excludes only persistence mechanics (`path`/file ownership, pending/live physical frame buffers and poison state).

The projection includes effects, branch/frontier state, sequencer ownership, memberships, quorum/election/decision authority, authenticated evidence indexes, recovery evidence/frontiers, quorum availability and publication state. `capture()` destructures `ReplicationAuthorityJournal` exhaustively without `..`, so adding a new journal field makes the R&D model fail to compile until that field is explicitly classified as semantic or physical-only.

### 2. Executable capture/restore property
The replication effect lifecycle regression now exercises:

```text
capture(replay(history))
    ==
capture(restore(capture(replay(history))))
```

The exercised state also asserts its retained-history shape. This makes omission of a semantic journal field visible to the R&D model instead of relying on an informal frame inventory.

### 3. Negative R&D result: snapshot-only compaction is asymptotically insufficient
Hostile inspection of production readers found that replicated `effects` and `revision_frontiers` are not replay garbage. They are consumed by causal validation and replicated-branch ideal construction. Historical prerequisite effects may therefore remain semantically reachable.

Consequently any exact monolithic semantic snapshot has an `Ω(retained replicated causal history)` lower bound in the worst case. Rewriting such a snapshot at every checkpoint still admits cumulative quadratic write work as history grows. A smaller frame set can remove redundant votes/promises, but cannot solve the fundamental class.

### 4. Architecture selected: immutable authority segments
Added `docs/architecture/REPLICATION_AUTHORITY_COMPACTION.md` specifying the replacement:

```text
AuthorityRoot
    -> Segment N(delta N, parent digest)
    -> Segment N-1
    -> ...
    -> semantic base segment
```

Each checkpoint must write only the new canonical authority delta plus a cryptographically bound stable parent identity. Normal operation must not recopy old history and must not fall back to a monolithic historical journal.

The required implementation law is:

```text
replay(flat historical frames)
    ==
replay(flatten(authority segment chain))
```

Replay remains the existing `ReplicationAuthorityJournal` semantics; the new physical structure is not a second authority evaluator.

### 5. Physical-location conclusion
A parent edge cannot be an absolute single-file offset. Current physical compaction relocates the active generation. Segment identity therefore has to be content-addressed (digest + logical segment identity), while an index maps logical identity to relocatable physical extents.

## Verification
- `kernel-durability --all-targets`: **189/189 PASS**, plus multiprocess suites PASS.
- targeted semantic snapshot round-trip regression: PASS.
- `kernel-durability` strict Clippy `-D warnings`: PASS.
- workspace `cargo check --all-targets` with `RUSTFLAGS=-D warnings`: PASS.
- workspace strict Clippy `--all-targets -- -D warnings`: PASS.
- `kernel-durability` rustdoc `-D warnings`: PASS.
- `cargo fmt --check`: PASS.

## Scope / honest status
P324 does **not** claim that replication copy amplification is closed. It rejects the naive snapshot-only design, establishes the exact semantic-state inventory, and selects the persistent-segment architecture that can actually remove the asymptotic payer. P323's composite old-archive copy path remains the production path until segment replay equivalence is executable.

## Next target — P325
Introduce `ReplicationAuthoritySegmentId`, bounded canonical segment envelopes and a relocatable segment index. Convert recovery to flatten the segment chain through the existing frame replay semantics and prove byte/history replay equivalence before deleting the monolithic archive-copy path.
