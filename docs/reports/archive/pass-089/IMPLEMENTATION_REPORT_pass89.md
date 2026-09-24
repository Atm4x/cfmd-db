# IMPLEMENTATION REPORT — Pass89

## Scope

Pass89 rebases historical #12 onto the frozen Pass88 durability architecture without replacing Pass87 authority-uncertainty handling, Pass88 canonical recovered-state migration, or the existing authoritative WAL path.

## Changed production files

### `crates/kernel-durability/src/lib.rs`

Adds the WAL substrate required by exact cut/tail mirroring:

- `FileRevisionWal::create_at_lsn` for a shadow suffix beginning at the cut's exact next LSN;
- seeded WAL recovery for cross-cut unresolved PREPAREs;
- exact-frame append with contiguous-LSN validation;
- PREPARE/COMMIT append variants returning the encoded frame used by the authoritative WAL;
- public `PreparedCutCapsule` and `StreamingCheckpointProgress` exports.

The ordinary WAL APIs remain compatibility wrappers around the frame-returning path.

### `crates/kernel-durability/src/store.rs`

Adds the production streaming-checkpoint state machine:

- checkpoint format v2 chunk roots while retaining v1 monolithic decode;
- manifest v3 binding `H`, `E`, first tail LSN, certified tail LSN and object checksums;
- exact `PreparedCutCapsule` encoding/decoding with prepare LSN and payload checksum;
- unpublished `StreamingCheckpointJob` with pinned canonical checkpoint bytes, chunk progress, shadow WAL and mirrored/durable watermarks;
- `begin_streaming_checkpoint[_with_chunk_size]`;
- bounded `write_streaming_checkpoint_chunks`;
- `finalize_streaming_checkpoint` with full replay/certificate verification before manifest publication;
- exact-frame mirroring from all store PREPARE/COMMIT paths;
- shadow barriers aligned with authoritative durability barriers;
- reopen from chunked checkpoint + capsule + arbitrary-LSN WAL suffix;
- publication-prefix validation that permits legal later WAL growth;
- orphan chunk/capsule generation recognition for safe generation allocation/compaction;
- hostile tests for cross-cut transactions, interleaved writes, chunk corruption and unpublished shadow failure.

Unpublished shadow/chunk failure marks the job failed but does not poison or revoke the published generation. Authority uncertainty begins only at manifest publication, preserving Pass87 semantics.

## Verification

Frozen-tree gate:

- `cargo fmt --all -- --check` — PASS;
- `cargo check --workspace --all-targets` — PASS;
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS;
- `cargo test --workspace --all-targets` — PASS;
- declared tests: **664**;
- failed: **0**;
- ignored: **8**.

Targeted durability checks also passed: all streaming-checkpoint hostile tests and the complete `kernel-durability` suite (**63 tests** at the Pass89 checkpoint).

Frozen source fingerprint over Rust/Cargo production inputs: `852fc52eb7551e34916ac1342ee472ed08c43372c33a20ec51932f15d895c52c`.

## Problem → hypothesis → implementation → falsification → result

**Problem:** chunking checkpoint bytes alone is insufficient because a PREPARE can occur before cut and COMMIT after it, and writes can continue while chunks are emitted.

**Hypothesis:** one immutable cut H plus exact unresolved-PREPARE capsule and byte-identical WAL suffix mirroring is sufficient to reconstruct any publication endpoint E reached during the job.

**Implementation:** pin H, capture capsule, seed an unpublished shadow at the exact next LSN, mirror exact authoritative frames, track durable shadow watermark, encode one chunk-root checkpoint, and publish only after replay proves E.

**Falsification:** force a cross-cut transaction, interleave commits with chunk writes, corrupt a chunk, fail the unpublished shadow, and write again after publication before reopen.

**Result:** all hostile cases behave according to the protocol; old authority survives every pre-publication failure, and a published generation recovers its certified E prefix plus legal later WAL commits.

## Engineering follow-up outside historical #12 closure

The canonical checkpoint encoder currently materializes the logical stream into memory before chunk emission. A future implementation may stream encoding/decoding directly into chunk files to lower peak memory and overlap CPU/I/O. This must preserve exactly the same cut/capsule/shadow/publication contract; it must not create a second authority model.
