# PASS319 REPORT — Bounded-memory encrypted generation publication

## Wall clock
- Start: 2026-09-29 01:32:27 UTC
- Functional freeze: 2026-09-29 01:52:27 UTC
- Useful boundary: 2026-09-29 01:52:27 UTC
- Hard boundary: 2026-09-29 01:56:27 UTC

## Goal
Remove the remaining payload-proportional ciphertext staging from single-file generation publication. P315 made encrypted section reads/copies chunk-bounded, but the writer still built every encrypted section as a whole `Vec<u8>` before generation layout. P319 must publish encrypted generations directly to their final file offsets with bounded additional memory, preserve the existing root/crash authority law, and avoid pre-release compatibility routing.

## Delivered

### 1. Removed whole-section ciphertext staging
The production `StoredSection { bytes: Vec<u8> }`, `prepare_stored_sections()` and whole-section `seal_chunked_section()` staging path are removed.

Generation layout now computes the exact stored length from plaintext length, fixed `CFSC` header size, chunk count and fixed AEAD envelope overhead without encrypting the section first.

Encrypted publication writes each 64 KiB plaintext chunk directly through AES-256-GCM-SIV to its final generation location. Only the current AEAD envelope is materialized; memory no longer grows with encrypted section size.

Unencrypted sections are also written directly from the caller slice instead of being copied into a second stored-section buffer.

### 2. Descriptor table moved to a generation footer
The old layout placed the section descriptor table before section payloads. That forced ciphertext section digests to exist before physical writing and therefore encouraged staging.

The current pre-release layout is:

```text
generation header
    -> deterministic page padding
    -> section 0 payload
    -> aligned section 1 payload
    -> ...
    -> section N payload
    -> section descriptor table footer
    -> final page padding
    -> WAL region
```

The fixed generation header now records the exact `section_table_offset` in addition to table length, `data_start` and `total_len`. Its header digest covers the footer offset. No format-version branch was added: CFMD is still pre-release and this is the one current generation layout.

### 3. One-pass physical generation digest
P319 does not trade RAM staging for a full post-write reread.

The generation SHA-256 is accumulated in exact physical byte order while the file is produced:

```text
header
+ prefix zero padding
+ section ciphertext/plaintext
+ alignment padding
+ descriptor footer
+ final padding
```

Each section has a separate digest accumulated over its final stored bytes at the same time. Once all section digests are known, footer descriptors are encoded one 64-byte record at a time directly into the file/generation hasher; there is no second table-sized `Vec<u8>`.

The root therefore receives the same kind of exact physical-generation digest without a second pass over a potentially huge generation.

### 4. Canonical footer validation
Reader/recovery now seeks to the footer offset from the generation header instead of assuming descriptors immediately follow the header.

Validation requires:
- `data_start` to equal the canonical first page after the fixed header;
- `section_table_offset` to be page-aligned and inside the generation;
- `total_len` to equal the canonical page-aligned end of footer table;
- every descriptor range to end before the footer;
- the footer to start exactly at the canonical aligned end of the final section, preventing an arbitrary hidden gap;
- the generation header digest and root generation digest to match as before.

The existing encrypted-section regression was extended to assert that the descriptor table is physically after section payloads and before the WAL boundary.

### 5. Crash/publication law unchanged
Generation bytes may be partially written after a crash, but they remain non-authoritative orphan bytes until:

```text
complete streamed generation write
    -> file sync
    -> root publication
    -> root sync
```

P319 changes physical construction, not database authority. Existing orphan-generation, torn-root, WAL rotation, compaction, external freshness and streaming-checkpoint regressions continue to pass.

### 6. Exact scope of the memory claim
P319 closes payload-proportional **additional physical encryption/storage-writer memory**. The writer itself now needs only:
- one bounded 64 KiB AEAD chunk envelope;
- section descriptors proportional to the explicitly bounded `MAX_SECTION_COUNT` (footer records themselves are emitted from a 64-byte stack buffer);
- fixed hash/padding buffers.

Higher canonical codecs still create caller-owned plaintext `Vec<u8>` values before invoking the physical writer: `checkpoint/revision_codec.rs::encode_revision`, `metadata/aggregate.rs::encode`, the single-file backend bootstrap/rotation call sites, and `streaming_checkpoint.rs::payload`. P319 intentionally does not hide that remaining payer.

## Verification
- `kernel-durability`: **182/182 PASS**
- encrypted chunk/footer targeted regression: PASS
- `cfmd-runtime`: **34/34 PASS**
- `cfmd-host`: **8/8 PASS**
- `kernel-plan`: **290 PASS / 5 ignored**
- `cfmd-protocol`: **4/4 PASS**
- `kernel-integration`: **21/21 PASS**
- strict Clippy `-D warnings` for `kernel-durability`, `kernel-plan`, `cfmd-runtime`, `cfmd-host`: PASS
- workspace `cargo check --workspace --all-targets --offline`: PASS
- strict rustdoc `-D warnings` for touched/public production boundaries: PASS
- `cargo fmt --all -- --check`: PASS
- Rust: supplied **1.98.1** toolchain

Lean 4.34.0 was not required. P319 does not alter the abstract publication authority/crash-cut law; it changes how a non-authoritative candidate generation is physically streamed before the existing generation-sync/root-publication boundary.

## Architectural conclusion
The physical single-file writer no longer has an encryption-specific whole-section memory path. Stored lengths are known algebraically before encryption, payloads are emitted exactly once to final offsets, ciphertext digests are produced while writing, and the descriptor table naturally follows the data that determines those digests. This removes the previous staging fallback rather than optimizing it.

The footer layout also avoids the alternative of rereading the entire generation after publication merely to compute a physical digest.

## Next target — P320
The hostile review exposed the next memory payer above the physical writer: `SingleFileSectionInput` still receives borrowed whole plaintext slices, while checkpoint/metadata canonical codecs commonly construct complete `Vec<u8>` payloads first.

P320 should introduce a one-pass exact-length section-source/canonical encoder contract:

```text
canonical codec
    -> exact encoded length
    -> streaming byte source
    -> CFMD AE 64 KiB chunker
    -> final generation offset
```

The byte-slice form should become an adapter over that one maintained path, not a parallel implementation. The goal is end-to-end payload-bounded publication for large checkpoints/metadata without requiring replay, temp files or a generic fallback.

After that, directory encryption parity can reuse the same AE v1/key hierarchy and streaming codec rather than building its own encryption implementation. Password-to-KEK/Argon2id remains a separate product adapter.
