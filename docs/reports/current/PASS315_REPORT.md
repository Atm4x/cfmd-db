# PASS315 REPORT — CFMD AE v1 productization: bounded sections / nonce namespace / KeyProvider

## Wall clock
- Start: 2026-09-28 23:50:29 UTC
- Functional freeze: 2026-09-29 00:10:17 UTC
- Useful boundary: 00:10:29 UTC
- Hard boundary: 00:14:29 UTC

## Goal
Close the first production payers left by P314 without creating a second encryption architecture: remove whole-section decrypt/copy materialization, remove per-WAL-record CSPRNG calls when a cheaper sound nonce namespace suffices, expose an external key-provider lifecycle, and make external-freshness-aware single-file open use the same encryption contract.

## Delivered

### 1. Bounded authenticated encrypted-section format
New encrypted single-file generation sections use a versioned `CFSC` stream with **64 KiB plaintext chunks**. Every chunk is independently wrapped by the existing CFMD AE v1 AES-256-GCM-SIV envelope.

Chunk AAD binds:
```
generation
section kind
section ordinal
chunk index
total plaintext length
chunk plaintext length
```

`SingleFileContainer::copy_section_to` now decrypts/authenticates one bounded chunk at a time. No whole encrypted section is materialized before streaming plaintext to the caller. Each chunk is authenticated before bytes from that chunk are released. The physical section SHA-256 digest is still verified over the exact stored chunk stream.

P314 whole-section `CFAE` sections remain explicitly readable for format compatibility. New encrypted writes always use `CFSC`; there is no error-driven fallback from the new writer to the old whole-section format.

`read_section()` still returns a `Vec` by API contract and therefore necessarily materializes its requested plaintext result; streaming callers use `copy_section_to`. Physical compaction already copies the authoritative encrypted generation byte-for-byte and therefore does not decrypt/re-encrypt sections.

### 2. WAL/section nonce namespace
The original P314 design called `getrandom` for every encrypted WAL frame and every encrypted section. P315 replaces that hot-path behavior with `StorageNonceSequence`:

```
nonce = random 80-bit namespace || u16 counter
```

A fresh namespace is obtained from the OS CSPRNG at writer/section-publication initialization and after every 65,536 emitted nonces. The counter is exact inside a namespace. AES-256-GCM-SIV nonce-misuse resistance remains defense in depth for the probabilistic cross-namespace collision case; this scheme is not presented as a substitute for long-lived key-epoch/rotation policy.

Retained microbenchmark: `artifacts/p315/NONCE_BENCHMARK.csv`.

On the current benchmark host, 5,000,000 nonce operations measured:
- per-record `getrandom`: **33.45 ns/nonce**;
- 80-bit namespace + u16 counter with namespace rotation: **0.612 ns/nonce**;
- nonce-generation microcost reduction: about **54.6x** / ~32.8 ns per record.

This is a nonce-generation microbenchmark, not an end-to-end fsync/commit benchmark.

### 3. Product `EncryptionKeyProvider`
`cfmd-runtime` now exposes:
- `EncryptionKeyProvider`;
- `EncryptionKeyOperation::{Create, Open}`;
- `Encryption::aes256_gcm_siv_with_provider(...)`.

`DatabaseBuilder` resolves provider-owned key material at the create/open lifecycle boundary and then enters the same typed kernel `StorageEncryption` path. Raw `EncryptionKey::from_bytes` remains the minimal adapter rather than the only product model.

P315 deliberately does **not** claim wrapped random database-master-key/password-KDF completion. The current provider still supplies the effective external 256-bit master material. Wrapped DMK + provider key IDs/epochs remain the next key-management layer.

### 4. External freshness + encryption parity
Low-level single-file create now has `create_single_file_with_encryption(...)` and external-freshness-aware reopen has `open_single_file_with_external_freshness_and_encryption(...)`.

Freshness preflight opens/authenticates encrypted metadata with the supplied storage-encryption configuration before deriving freshness material. An encrypted externally anchored store cannot fall through a plaintext freshness probe.

Regression coverage verifies:
- missing encryption on freshness-aware open fails;
- keyed ordinary open still refuses an externally anchored store;
- keyed freshness-aware open succeeds and restores freshness authority.

### 5. Hostile/R&D correction during the pass
An initial session-nonce prototype used SHA-256 over a random session seed + counter. Measurement showed it was **slower** than Linux `getrandom` on this host (~112.6 ns versus ~34.7 ns per nonce), so it was rejected rather than retained as an architectural abstraction. The final namespace/counter construction measured ~0.612 ns per nonce and is the implementation in the repository.

This is exactly the intended R&D policy: a proposed abstraction that loses to the existing primitive is removed, not kept as a fallback or conceptual layer.

## Verification completed before/at freeze
- `cargo check -p kernel-durability -p kernel-plan -p cfmd-runtime -p cfmd-host --offline` with `RUSTFLAGS=-D warnings`: **PASS**
- `cargo test -p kernel-durability --offline --lib`: **178/178 PASS**
- `cargo test -p cfmd-runtime --offline`: **31/31 PASS**
- `cargo test -p cfmd-host --offline`: **8/8 PASS**
- encrypted >128 KiB multi-chunk section stream-copy/reopen regression: **PASS**
- encrypted external-freshness keyed-open regression: **PASS**
- provider-backed create/open regression: **PASS**
- nonce namespace rollover regression: **PASS**
- `cargo fmt --all -- --check` using the supplied Rust 1.98.1 rustfmt component: **PASS**

Lean was not required by the P315 change: no Lean model/refinement binder was modified. The supplied Lean 4.34.0 split archives were therefore left untouched.

## Remaining payers
1. **Wrapped database master key**: create a random per-database DMK, wrap it under provider/KEK material, persist provider key ID + wrap metadata, and make password changes/key-provider rotation rewrap rather than re-encrypt the database.
2. **Key epochs / rotation**: persist explicit crypto epoch vocabulary and define section/WAL rekey migration without algorithm fallback.
3. **Fully streaming encrypted publication**: read/copy is bounded, but generation publication still constructs stored encrypted section bytes before writing the generation table/digest. If very large section creation becomes a product requirement, replace this with a two-pass or bounded staging layout without temp-file authority.
4. **Directory encryption parity**: still fail-closed. Do not add a second encryption implementation; route directory physical objects through the same AE codec/key hierarchy when product demand justifies it.
5. **End-to-end WAL performance evidence**: nonce microcost is closed, but a dedicated WAL benchmark should separate AEAD+encoding from fsync/storage latency before further optimization.

## Architectural conclusion
P315 keeps CFMD AE v1 as one storage protocol rather than a collection of encryption routes. Section granularity is now bounded/authenticated, WAL nonce generation no longer pays one CSPRNG call per frame, key acquisition is provider-extensible, and freshness-aware recovery uses the same keyed boundary. AES-256-GCM-SIV remains the only production primitive; crypto agility remains persisted vocabulary, not runtime fallback.

## Next target — P316
Implement the **wrapped database-master-key lifecycle** cleanly:
1. random per-database DMK generated at create;
2. provider/KEK wraps the DMK; only wrapped DMK + provider key identifier/epoch are persisted;
3. open resolves the provider KEK, authenticates/unlocks the DMK, then derives AE v1 domain keys;
4. key-provider/password rotation rewraps the DMK without rewriting all database ciphertext;
5. define fail-closed key epoch/rotation/recovery laws and add crash tests around wrap-metadata publication.

Do not add another AEAD in P316 unless the key-management architecture itself requires algorithm vocabulary changes.
