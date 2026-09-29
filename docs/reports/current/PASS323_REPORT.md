# PASS323 REPORT — Bounded composite replication-authority publication

## Wall clock
- Start: 2026-09-29 05:12:05 UTC
- Functional freeze: 2026-09-29 05:31:45 UTC
- Useful boundary: 05:32:05 UTC
- Hard boundary: 05:36:05 UTC

## Goal
Remove the remaining whole-archive `Vec<u8>` construction from SingleFile checkpoint rotation. Preserve the exact replication-authority cut while composing the previous authoritative archive with only the frozen live-frame prefix through the same bounded section-source path introduced by P319–P322.

## Delivered

### 1. First-class section snapshot source
`SingleFileContainer::section_source_snapshot(...)` captures the current authoritative section descriptor, generation, crypto context and an independent read handle. `SingleFileSectionSnapshot` implements the existing `SingleFileSectionSource` contract and emits plaintext through one bounded 64 KiB buffer.

The independent handle is opened when the snapshot is captured rather than reopening the pathname during publication. This avoids both shared file-offset interference with the append writer and a pathname TOCTOU between planning and source consumption.

Encrypted archived sections are authenticated chunk-by-chunk by the existing `SingleFileSectionReader` before plaintext is emitted. Stored-section digest verification still completes before the source is accepted as fully consumed.

### 2. Composite replication archive source
`ReplicationArchiveSectionSource` defines the successor replication section as the exact concatenation:

```text
previous authoritative ReplicationAuthority section
||
first N live single-file replication frames
```

It computes an exact plaintext length without concatenation and streams each component directly into the generation writer. The old helpers that built `single_file_replication_archive()` / `single_file_replication_archive_prefix()` as complete vectors were removed.

### 3. Ordinary rotation is archive-size bounded in RAM
Synchronous SingleFile checkpoint rotation now snapshots the previous replication section, freezes the current live-frame count, and gives the composite source directly to `SingleFileSectionInput::streaming(...)`.

No whole old archive, live prefix or concatenated successor archive is allocated.

### 4. Streaming checkpoint no longer duplicates the frozen archive
`StreamingCheckpointPhysical::SingleFile` no longer stores `replication_archive: Vec<u8>`. Begin records only `replication_cut_frames`.

Finalize reconstructs the exact cut from:
- the still-authoritative previous generation section; and
- the first `replication_cut_frames` live frames.

Frames appended after begin are therefore excluded from the checkpoint section and remain the live suffix exactly as before. After publication, `advance_single_file_generation_prefix(replication_cut_frames)` retires only the captured prefix.

### 5. Encrypted archived-source regression
A new regression creates an encrypted SingleFile database, publishes replication membership authority, rotates twice, and reopens with the same key. This specifically exercises decrypt/authenticate -> bounded composite source -> re-encrypt under the successor generation AAD across multiple rotations.

### 6. Hostile follow-up: copy amplification remains
P323 closes archive-sized memory duplication, not historical rewrite cost. A successor generation still contains the complete retained replication-authority archive. Repeated rotations therefore pay O(retained replication history) physical work per rotation.

This is now an explicit R&D obligation rather than being hidden behind a buffering implementation: the next improvement must derive a compact canonical replication-authority snapshot/delta law proving which state replaces old frames. Dropping history without such a law is not acceptable.

### 7. Documentation
`PROJECT_STATUS.md` and `CFMD_CORE_SPEC.md` now record the P320–P323 bounded source/sink progression and the remaining replication copy-amplification obligation.

## Acceptance / gates
- `kernel-durability`: **189/189 PASS**, including multiprocess suites.
- `cfmd-runtime`: **34/34 PASS**.
- `cfmd-host`: **8/8 PASS**.
- `cfmd-protocol`: **9/9 PASS**.
- encrypted replication archive streaming across repeated rotations: **PASS**.
- ordinary replication rotation/reopen regression: **PASS**.
- streaming checkpoint exact WAL + replication suffix regression: **PASS**.
- `cargo check --workspace --all-targets --offline`: **PASS**.
- strict Rust check (`RUSTFLAGS=-D warnings`) for `kernel-durability --all-targets`: **PASS**.
- strict workspace Clippy (`-D warnings`): **PASS**.
- strict `kernel-durability` rustdoc (`-D warnings`): **PASS**.
- `cargo fmt --all -- --check`: **PASS**.
- repository verifier: **PASS**.
- repository manifest: **3401 files**.
- packaged repository excludes build `target/`: **PASS**.

Lean was not required: P323 changes how already-defined replication-authority bytes are sourced during candidate generation construction. Root/generation/WAL publication authority and formal crash cuts are unchanged.

## Remaining hostile inventory
The next two concrete payload/cost payers are:

1. replication authority still has repeated-history physical copy amplification across rotations;
2. prepared-cut capsule write/read paths still materialize the complete capsule (`encode_prepared_cut_capsule -> Vec<u8>` and SingleFile `read_section -> Vec<u8>`).

## Next target — P324
R&D a compact canonical replication-authority snapshot/delta representation that preserves every authority obligation currently reconstructed from the journal while making checkpoint rotation proportional to current authority state plus new delta, rather than all historical frames. The current journal has distinct authority families (effects/publication, membership and membership-vote ownership, term/leader election, decision votes/locks, quorum availability/recovery, peer-authenticated evidence and recovery frontiers), so P324 must prove snapshot sufficiency per family rather than collapse them into one generic state dump. Prove equivalence through replay/snapshot regressions and keep the journal form only where history is semantically required. If that calculus cannot safely compact every authority class in one pass, explicitly retain the irreducible subset rather than using a generic fallback.

After that, finish prepared-cut capsule source/sink streaming and proceed to Directory AE v1 parity.
