# PASS321 REPORT — Resumable bounded checkpoint source / directory streaming memory closure

## Wall clock
- Start: 2026-09-29 02:23:58 UTC
- Functional freeze: 2026-09-29 02:43:58 UTC
- Useful boundary: 2026-09-29 02:43:58 UTC
- Hard boundary: 2026-09-29 02:47:58 UTC

## Goal
Continue P320's end-to-end bounded canonical I/O work without introducing a second serializer or an O(number-of-chunks × checkpoint-size) re-encoding fallback. The first P321 target is the remaining Directory resumable streaming-checkpoint `Vec<u8>`: preserve the existing cross-call chunk budget/WAL-shadow protocol while removing checkpoint-sized RAM retention.

## Delivered

### 1. Directory resumable checkpoint no longer owns the encoded checkpoint in RAM
The production `store/` path now has **zero** `checkpoint::encode_revision(...)` calls. `StreamingCheckpointJob` no longer contains `payload: Option<Vec<u8>>`.

Directory streaming checkpoint uses a generation-scoped `CheckpointSpool` which is a transient canonical byte source, not durability authority:

```text
immutable Revision
      -> exact canonical length/count pass
      -> canonical streaming encoder
      -> checkpoint-...-stream.tmp   (scratch, unpublished)
      -> bounded 64 KiB copy buffer
      -> chunk-N.cfck
      -> existing chunk CRC/root protocol
      -> manifest publication
```

This keeps the existing API semantics: `write_streaming_checkpoint_chunks(max_chunks)` still performs only the requested chunk publication work. It does not pre-create all durable chunk sidecars during `begin_streaming_checkpoint`.

### 2. Linear-time resumability instead of a hidden O(n²) fallback
A tempting implementation would re-run the canonical encoder from byte zero for every requested chunk and discard the already-published prefix. That would make a many-chunk checkpoint O(n²).

P321 explicitly does **not** do this. The canonical checkpoint is encoded once into a sequential scratch source, then each later chunk is read from its exact offset. Total encoding/copy work remains O(checkpoint bytes), with memory bounded by the existing 64 KiB codec buffer plus one 64 KiB spool-copy buffer.

Single-file streaming was kept on its existing direct canonical path and does not pay a new count+spool pass.

### 3. Scratch bytes are cryptographically bound to the original canonical stream
Moving the immutable snapshot from RAM to scratch introduces a new hostile surface: the scratch file could be changed between `begin` and later chunk writes.

P321 therefore hashes the exact canonical stream with SHA-256 while creating the spool and incrementally hashes the exact bytes consumed during chunk publication. Before the checkpoint root becomes eligible for publication, the two digests must match.

A modified spool fails closed with:

```text
checkpoint canonical spool changed during resumable publication
```

and the old published generation remains authority.

### 4. Failed chunk I/O makes the job non-resumable
The spool digest is stateful across chunk calls. An I/O failure after consuming bytes cannot safely be retried as though nothing happened because that would double-consume digest state and may leave a partial candidate chunk.

P321 therefore marks the streaming job failed on chunk creation/copy/sync failure. This is an explicit state transition, not an accidental retry path.

### 5. Scratch lifecycle is non-authoritative and crash-clean
The spool is deliberately **not fsync'd as authority**. Only the existing chunk/root/manifest barriers create durable authority.

Lifecycle rules:
- normal success/error/drop closes the file before deletion (including Windows semantics);
- generation allocation recognizes orphan `checkpoint-...-stream.tmp` names so a crash cannot cause generation-number reuse;
- Directory recovery, while holding the directory lock, removes orphan stream spools from dead processes;
- ordinary generation compaction also recognizes/removes stale stream-spool artifacts.

No recovery path ever reads a spool as database authority.

### 6. Regression coverage
Added regressions prove:
- Directory streaming creates a canonical spool instead of a checkpoint-sized in-memory payload;
- chunk files are still produced incrementally only when chunk budget is consumed;
- the spool's exact physical length equals canonical encoded length;
- finalization removes the scratch source and reopen recovers the published checkpoint;
- spool tampering is rejected before root publication and previous authority survives;
- recovery removes an orphan spool left by a crashed/unpublished job.

## Hostile findings / remaining payers
P321 closes priority (1) from P320. The remaining payload-sized paths are now sharply localized:

1. Single-file open/recovery still uses `read_section(...) -> Vec<u8>` for checkpoint and metadata before decode. The clean next step is a pull-based `BinarySource`/bounded section reader, not callback buffering. Inventory: 45 checkpoint/metadata decoder signatures are slice-`Cursor` coupled, but only 6 sites directly require `take(...)` ownership/borrowing adaptation.
2. Single-file replication-authority generation rotation still concatenates existing archive + live frames into `Vec<u8>` (`single_file_replication_archive{,_prefix}`). This needs a first-class composite section source or carried-section reader, not another temporary full archive buffer.
3. Freshness probing also decodes single-file metadata through a materialized section and should reuse the same bounded reader introduced for (1).

Prepared-capsule materialization remains an explicitly bounded authority object and is not conflated with these large-payload payers.

## Verification
- `kernel-durability`: **186/186 PASS** including multiprocess suites
- `cfmd-runtime`: **34/34 PASS**
- `cfmd-host`: **8/8 PASS**
- targeted Directory resumable/checkpoint suite: PASS
- spool lifecycle/tamper regressions: PASS
- workspace strict Clippy `--workspace --all-targets -- -D warnings`: PASS
- strict `kernel-durability` rustdoc `-D warnings`: PASS
- `cargo fmt --all -- --check`: PASS
- Rust: supplied **1.98.1** toolchain

Lean 4.34.0 was not required. P321 changes unpublished Directory scratch/candidate byte production only; root/manifest publication authority and modeled crash cuts are unchanged.

## Architectural conclusion
The Directory resumable checkpoint is now bounded-memory without re-encoding prefixes and without weakening the existing chunk/WAL-shadow publication protocol. The important distinction is explicit: the spool is **resumable scratch state**, never recovery authority. Authority remains exactly the already-certified chunk/root/manifest chain.

## Next target — P322
Close bounded **read/recovery** first, because that unlocks both ordinary open and freshness probing with one maintained abstraction:

```text
single-file section
   -> bounded plaintext reader (AEAD chunk authentication before release)
   -> generic BinarySource
   -> checkpoint / metadata canonical decoder
   -> typed Revision / DurableStoreMetadata
```

The implementation should generalize the existing slice `Cursor` rather than add a second decoder grammar. After that, use the same section-reader/source machinery to remove the replication-archive-sized concatenation `Vec<u8>`. Only then should Directory AE v1 parity be layered over the common bounded read/write pipeline.
