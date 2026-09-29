# PASS322 REPORT — Bounded single-file recovery / pull-based canonical decode

## Wall clock
- Start: 2026-09-29 02:47:26 UTC
- Functional freeze: 2026-09-29 03:07:26 UTC
- Useful boundary: 2026-09-29 03:07:26 UTC
- Hard boundary: 2026-09-29 03:11:26 UTC

## Goal
Close the large-payload read-side counterpart of P319-P321. Single-file checkpoint/metadata recovery must not materialize an entire section into `Vec<u8>` before canonical decode, and encrypted sections must authenticate each bounded AEAD chunk before releasing its plaintext. Reuse one canonical decoder grammar rather than maintain a slice decoder plus a second streaming decoder.

## Delivered

### 1. One canonical decode grammar over a `BinarySource`
`binary_codec` now has a pull-based `BinarySource` contract. Existing slice `Cursor` and the new `ReadBinarySource` implement the same primitive/collection/value operations.

Checkpoint and metadata decoder functions were generalized over `impl BinarySource`; they are statically dispatched rather than routed through a `dyn BinarySource` vtable. Existing slice decode remains a thin source adapter, while reader decode uses the exact same grammar.

Fixed-width primitives decode into stack arrays. Variable-length owned fields allocate only their declared, bounded payload. No per-field `Vec` is created for u8/u16/u32/u64/u128 reads.

### 2. Bounded single-file plaintext reader
`SingleFileSectionReader` is a pull reader over one generation section.

For plaintext sections it reads/hash-validates in bounded 64 KiB windows instead of issuing a physical file read for every decoder primitive. For encrypted `CFSC` sections it:
- validates the physical section header and exact stored length;
- reads at most one AEAD envelope at a time;
- authenticates/decrypts that chunk before exposing any plaintext from it;
- binds decryption to the existing generation/kind/ordinal/chunk-index/length AAD;
- verifies the stored section digest and exact plaintext consumption.

Zero-length encrypted sections still consume/authenticate their mandatory zero-length AEAD envelope before successful finish.

### 3. Checkpoint and metadata open/recovery are no longer section-sized
`DurableRevisionStore::open_single_file_inner` now decodes durable metadata and checkpoint directly from `SingleFileSectionReader` through the common canonical `BinarySource` grammar.

The production single-file store path now has zero `read_section(Checkpoint, ...)` or `read_section(Metadata, ...)` calls.

### 4. External freshness probe uses the same bounded metadata path
Single-file external-freshness preflight no longer materializes metadata either. The freshness probe uses the same authenticated section reader and metadata decoder as ordinary open, eliminating a second read architecture.

### 5. Replication archive recovery is bounded too
Single-file replication authority open now accepts an archived `Read` source plus exact section length. Archive replay reads one frame header and one bounded frame at a time, then applies it immediately. Ordinary single-file open therefore no longer constructs a full archived-replication `Vec<u8>` just to replay it.

Live post-generation frames retain their existing bounded per-frame representation and replay semantics.

### 6. Regression coverage
Added decoder regressions proving checkpoint canonical decode works over a deliberately fragmented pull reader and metadata decode works through the reader source without requiring a slice.

Existing encrypted single-file/freshness tests also exercise the new section-reader path. A zero-length encrypted replication-authority section exposed a real finish-path bug during hostile verification; P322 fixes it by authenticating the empty AEAD envelope rather than treating zero plaintext as equivalent to zero stored bytes.

## Hostile findings / remaining payers
The major single-file read-side checkpoint/metadata materialization is closed. Remaining production `read_section(...)` calls are now limited to:
- prepared capsule materialization;
- replication-authority **generation rotation** helpers that still concatenate previous archive + live frames into a `Vec<u8>`.

Replication rotation is now the clear next payload-sized payer. The correct next architecture is a first-class composite `SingleFileSectionSource`: stream the previous authoritative replication section from a read-only snapshot/source and append the frozen live-frame prefix directly into the new generation writer. The streaming-checkpoint job must preserve its exact replication cut without storing one monolithic archive blob.

Prepared capsule is lower priority because its logical state is already represented by the in-memory prepared ledger and each encoded prepare payload is hard-bounded, but its single-file byte materialization can later be removed through the same source/reader machinery.

## Verification
- `kernel-durability`: **188/188 PASS**, including multiprocess suites
- `cfmd-runtime`: **34/34 PASS**
- `cfmd-host`: **8/8 PASS**
- `cfmd-protocol`: existing suite PASS
- checkpoint fragmented-reader regression: PASS
- metadata reader-source regression: PASS
- encrypted external-freshness keyed-open regression: PASS
- workspace strict Clippy `--workspace --all-targets -- -D warnings`: PASS
- strict `kernel-durability` rustdoc `-D warnings`: PASS
- `cargo check --workspace --all-targets --offline`: PASS
- `cargo fmt --all -- --check`: PASS
- Rust: supplied **1.98.1** toolchain

Lean 4.34.0 was not required. P322 changes bounded byte consumption and canonical decode plumbing; root/generation/WAL publication authority and modeled crash cuts are unchanged.

## Architectural conclusion
The normal single-file recovery path is now bounded in both directions:

```text
Revision / Metadata
        ↓ write
canonical BinarySink
        ↓
bounded section stream
        ↓
AEAD / physical storage
        ↓ read
bounded authenticated section reader
        ↓
canonical BinarySource
        ↓
Revision / Metadata
```

There is one maintained canonical grammar on each direction; slice APIs are adapters, not fallback implementations.

## Next target — P323
Remove replication-archive-sized staging during single-file generation rotation and streaming-checkpoint cut capture. Build a composite/carrying section source that streams the old archived authority plus an exact frozen live-frame prefix directly into the P319 writer, with no whole-archive `Vec<u8>` and no O(n²) replay/re-encoding. Then reassess the prepared-capsule byte materialization before starting Directory AE v1 parity.
