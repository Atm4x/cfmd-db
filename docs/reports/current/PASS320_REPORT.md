# PASS320 REPORT — Streaming canonical encoding / section-source publication

## Wall clock
- Start: 2026-09-29 01:58:59 UTC
- Functional freeze: 2026-09-29 02:18:59 UTC
- Useful boundary: 2026-09-29 02:18:59 UTC
- Hard boundary: 2026-09-29 02:22:59 UTC

## Goal
Remove the plaintext-sized checkpoint/metadata `Vec<u8>` payer above the P319 bounded physical writer. Canonical encoders must be able to provide an exact encoded length and stream the exact same canonical bytes into one maintained single-file section-source path, so the default single-file product lifecycle is payload-bounded from logical Revision/metadata through AEAD publication.

## Delivered

### 1. Canonical binary codecs are sink-based
`binary_codec` now has one internal `BinarySink` contract instead of hard-wiring checkpoint/metadata encoders to `Vec<u8>`.

The existing buffered API remains only where callers genuinely need owned bytes; the same encoder implementation now also supports:

```text
CountingBinarySink
    -> exact canonical encoded length

StreamingBinarySink (64 KiB bounded buffer)
    -> canonical byte stream
```

Checkpoint/schema/state/value and metadata/artifact/intent/query/model-delta encoders were mechanically generalized over that one sink contract. There is no second serializer and no alternate wire grammar.

A regression proves that streamed checkpoint encoding is byte-for-byte identical to the buffered canonical encoding and reports the exact same length.

### 2. First-class single-file section sources
`SingleFileSectionInput` is no longer intrinsically `&[u8]`-only. The physical writer accepts either:

```text
Bytes(&[u8])
Streaming(&dyn SingleFileSectionSource)
```

Both variants flow through the same generation layout, digest, alignment, AEAD chunking and publication implementation. The byte-slice form is therefore an adapter over the maintained section path, not a fallback implementation.

A source declares its exact plaintext length and emits canonical bytes through a bounded callback. The physical writer verifies that the number of emitted bytes exactly matches the declared length and fails closed if a source changes between length planning and publication.

### 3. AEAD writer consumes arbitrary streaming fragments
The P319 `CFSC` writer no longer assumes a complete plaintext slice. It accepts arbitrary source fragment boundaries, accumulates at most one 64 KiB plaintext AEAD chunk, seals it with AES-256-GCM-SIV, and writes the envelope directly to the final generation offset.

For unencrypted sections the same source is written directly into the file/section/generation hashers without an intermediate payload buffer.

Generation layout keeps only bounded metadata: exact plaintext lengths and section descriptors. No checkpoint/metadata plaintext or ciphertext is staged proportionally to payload size.

### 4. Product single-file bootstrap and rotation are end-to-end streaming
The ordinary single-file create path and synchronous generation rotation no longer call:

```text
checkpoint::encode_revision(...) -> Vec<u8>
metadata::encode(...)            -> Vec<u8>
```

They instead install typed `RevisionSectionSource` / `MetadataSectionSource` adapters:

```text
Revision / DurableStoreMetadata
        -> exact canonical length
        -> canonical streaming encoder
        -> SingleFileSectionSource
        -> 64 KiB CFSC AEAD chunker
        -> final generation offsets
```

Prepared-capsule and replication-authority bytes remain separate bounded/archive concerns and are not falsely described as solved by the checkpoint codec refactor.

### 5. Single-file streaming-checkpoint job no longer retains encoded checkpoint payload
Previously `begin_streaming_checkpoint` always created and retained `payload: Vec<u8>`, even for the single-file backend, although single-file chunk-progress calls never physically wrote those in-memory chunks.

P320 makes the distinction explicit:
- single-file: retains the immutable `Revision`, exact encoded length and readiness state; no encoded checkpoint `Vec<u8>` is stored;
- directory chunked checkpoint: retains the current encoded payload because that backend's resumable chunk-by-chunk sidecar protocol still slices it across later API calls.

At single-file finalization the pinned immutable cut is encoded directly through `RevisionSectionSource`. Metadata is likewise retained as its typed record rather than pre-encoded bytes.

The remaining directory streaming-job payload is now the only production `checkpoint::encode_revision(...)` call under `store/` (apart from tests), making the next payer explicit rather than hidden among multiple paths.

### 6. Synchronous directory checkpoint/metadata publication is also streaming
The ordinary directory `write_checkpoint_file` and `write_metadata_file` paths no longer materialize complete encoded payloads.

They perform a bounded canonical pre-pass to determine length/checksum, then stream the exact canonical bytes directly into the new file while accumulating the whole-file CRC. This preserves the existing on-disk format and durability barriers while removing payload-sized writer memory.

The resumable *streaming-checkpoint* directory protocol is intentionally not rewritten into a fake one-shot path; making that API resumable without retaining a full payload requires a resumable canonical encoder/source contract and is left visible for the next pass.

### 7. No publication-law or format-version change
P320 changes byte production, not authority:

```text
canonical source
    -> candidate generation/file bytes
    -> durability barrier
    -> existing root/manifest publication
```

No `FORMAT_VERSION` bump, migration branch, legacy serializer or AEAD alternative was introduced. CFMD remains pre-release and has one current maintained physical layout.

## Hostile findings / remaining payers
The hostile inventory after P320 shows:

1. `store/streaming_checkpoint.rs` directory mode still owns `Option<Vec<u8>>` for the resumable checkpoint payload. This is now backend-specific and the only production store-level checkpoint materialization.
2. Single-file **recovery/read** still materializes checkpoint and metadata sections through `read_section(...) -> Vec<u8>` before decode. P320 closes publication memory, not recovery memory.
3. Single-file replication-authority archive rotation still returns/concatenates `Vec<u8>` and can become payload-sized independently of checkpoint encoding.
4. Prepared-capsule bytes remain materialized, but this is a separate explicitly bounded authority structure rather than a hidden copy of the full database revision.

## Verification
- `kernel-durability`: **183/183 PASS** (including multiprocess suites)
- new streaming canonical checkpoint byte/length parity regression: PASS
- `cfmd-runtime`: **34/34 PASS**
- `cfmd-host`: **8/8 PASS**
- `cfmd-protocol`: **9/9 PASS** across its two test targets (5 + 4)
- `kernel-plan`: **290 PASS / 5 ignored**
- `kernel-integration`: **21/21 PASS**
- workspace strict Clippy `--workspace --all-targets -- -D warnings`: PASS
- strict `kernel-durability` rustdoc `-D warnings`: PASS
- `cargo fmt --all -- --check`: PASS
- Rust: supplied **1.98.1** toolchain

Lean 4.34.0 was not required. P320 does not change root/generation/WAL publication authority or any modeled crash cut; it changes canonical byte production before the already-modeled durability/publication boundary.

## Architectural conclusion
The default single-file product path now has one canonical encoder and one physical section-source path from typed revision/metadata objects to final AEAD ciphertext. Large checkpoint/metadata publication no longer requires a same-sized plaintext `Vec<u8>` before the bounded P319 writer can begin.

The important architectural property is not merely "streaming": exact-length planning and byte emission are two views of the *same canonical encoder*. This prevents drift between size calculation, buffered encoding and physical storage encoding.

## Next target — P321
Close the remaining payload-sized authority I/O instead of adding another encryption algorithm.

Priority order:

```text
1. resumable canonical checkpoint source
   -> directory streaming-checkpoint no full payload Vec

2. streaming canonical decode / bounded section reader
   -> single-file open/recovery no full checkpoint/metadata Vec

3. replication-authority archive source
   -> generation rotation no archive-sized concatenation Vec
```

The desired result is a bidirectional bounded pipeline:

```text
Revision <-> canonical codec <-> bounded section stream <-> AEAD/storage
```

Only after those remaining generic memory payers are closed should directory encryption parity reuse the exact same CFMD AE v1 codec/key hierarchy. There is still no reason to add a second AEAD at this stage.
