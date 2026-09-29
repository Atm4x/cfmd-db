# PASS314 REPORT — CFMD AE v1 / AES-256-GCM-SIV foundation

## Wall clock
- Start: 2026-09-28 23:26:58 UTC
- Functional freeze: 2026-09-28 23:46:26 UTC
- Useful boundary: 23:46:58 UTC
- Hard boundary: 23:50:58 UTC

## Goal
Begin product encryption-at-rest without creating a second durability architecture. Introduce a crypto-agile CFMD AE v1 storage boundary over the existing single-file generation/WAL protocol, select a production baseline primitive from the preceding R&D benchmark, and expose encryption through the unified DatabaseBuilder/runtime path while preserving fail-closed crash/publication semantics.

## Primitive decision
CFMD AE v1 baseline is **AES-256-GCM-SIV** via RustCrypto `aes-gcm-siv 0.12.1`.

The retained benchmark evidence is `artifacts/p314/AEAD_BENCHMARK_RESULTS.csv`. On the benchmark host, AES-256-GCM-SIV reached about 3.30 GiB/s native and 2.51 GiB/s portable-generic at 1 MiB payloads. XChaCha20-Poly1305 was about 1.44 GiB/s native. AEGIS-256 with the bundled C backend was much faster (~9.77 GiB/s native at 1 MiB), but the current pure-Rust portable path collapsed to ~0.128 GiB/s. The product baseline therefore favors a pure-Rust, portable, nonce-misuse-resistant implementation; AEGIS remains a future optional/R&D candidate rather than a dependency of CFMD AE v1.

## Delivered

### 1. Storage encryption primitive boundary
`kernel-durability` now owns a dedicated `storage_encryption` module with:
- `StorageAeadAlgorithm` (currently `Aes256GcmSiv`);
- zeroizing/redacted `StorageEncryptionKey`;
- `StorageEncryption::{None, Aes256GcmSiv}`;
- `StorageAeadCodec` with domain-separated section/WAL operation;
- versioned `CFAE` authenticated envelope.

The algorithm identifier is persisted as protocol metadata, so later algorithms can be added without turning durability backends into algorithm-specific implementations. Unknown/unsupported algorithms fail closed; there is no plaintext fallback.

### 2. Key hierarchy and database binding
An externally supplied 256-bit master key is never written to the database file. New encrypted single-file stores generate and persist a random 256-bit database salt. HKDF-SHA256 derives distinct keys for:
- immutable generation sections;
- WAL records;
- key commitment.

The file header persists only public encryption metadata, salt, and a truncated commitment used to reject a wrong key before ordinary recovery. Raw master/derived keys are not persisted.

### 3. Single-file format v3
The single-file format version is bumped to v3. Header digest coverage now includes encryption metadata. Encrypted and plaintext stores are explicitly distinguished.

Opening an encrypted store without a key, opening it with the wrong key, or supplying encryption to a plaintext store fails explicitly. A stored AE envelope encountered on a plaintext path is treated as protocol/corruption rather than silently exposed as payload.

### 4. Immutable generation section AEAD
Each stored section is encrypted independently with a fresh 96-bit nonce under the section-derived AES-256-GCM-SIV key.

Section AAD binds stable physical identity:
```
generation || section_kind || ordinal
```
plus the CFMD AE v1 domain/version framing owned by the codec.

Existing generation/section digests cover the exact stored ciphertext bytes. Authentication/decryption occurs only after physical digest verification.

### 5. WAL AEAD
Each WAL frame payload is independently encrypted before the existing frame CRC/physical envelope is written. WAL AAD binds:
```
record_kind || LSN || Revision
```
under a separately derived WAL key.

Recovery verifies physical frame integrity first, then AEAD authentication, then decodes semantic payload. Freshness evidence continues to hash exact stored bytes, so encrypted WAL remains compatible with the existing freshness protocol.

Torn-tail recovery may reuse a logical LSN after discarding an incomplete frame without catastrophically reusing a successful AEAD `(key, nonce)` pair because each newly written frame receives a fresh nonce; AES-GCM-SIV additionally supplies nonce-misuse resistance rather than relying on nonce discipline as the only safety barrier.

### 6. Crash/publication law preserved
P306-P313 publication authority is unchanged. Carried WAL and compaction may preserve already-encrypted bytes exactly; encryption does not create a new recovery authority.

An AEAD authentication failure is corruption. It is never interpreted as permission to roll back to an older acknowledged root.

Whole-file rollback remains a separate external-freshness obligation; AEAD is not misrepresented as freshness/rollback protection.

### 7. Product/runtime path
`kernel-plan` now has `RuntimeStorageOptions { backend, encryption }`. Existing backend entry points remain compatibility wrappers over this unified configuration rather than gaining another family of `_and_encryption` methods.

`cfmd-runtime` exposes typed `Encryption` / `EncryptionKey` and `DatabaseBuilder::encryption(...)`:
```
Database::builder(path)
    .schema(schema)
    .encryption(Encryption::aes256_gcm_siv(key))
    .create()?;
```

Opening uses the same configuration path. Encryption is currently implemented for `SingleFile`; requesting encryption with `Directory` fails closed rather than routing to plaintext.

Hosting authentication/authorization remains above an already-open `Database`; it is not conflated with at-rest encryption credentials.

### 8. P313 hosting cleanup
The actual `DatabaseHostingExt::host` implementation still consumed `Database` despite the P313 report specifying a borrowed handle. P314 corrected the active API to `&self`, matching `host_with_limits` and the intended embedded+hosted coexistence model.

### 9. Dependency closure
The repository vendors the exact new pure-Rust crypto closure required for AES-256-GCM-SIV, HKDF-SHA256 and OS randomness. No OpenSSL/libsodium/system crypto runtime dependency is introduced. `THIRD_PARTY.md` was regenerated from the actual Cargo metadata closure.

## Verification completed before functional freeze
- `cargo check -p kernel-durability --offline`: PASS
- `cargo check -p kernel-plan --offline`: PASS
- `cargo check -p cfmd-runtime --offline`: PASS
- `cargo test -p kernel-durability --offline`: **175/175 PASS**
- `cargo test -p cfmd-runtime --offline`: **30/30 PASS**
- `cargo test -p cfmd-host --offline`: **8/8 PASS**
- encrypted single-file create/commit/raw-ciphertext/missing-key/wrong-key/exact-key reopen E2E: PASS (included in cfmd-runtime 30/30)

The supplied Rust 1.98.1 toolchain does not include `cargo-clippy`, so strict Clippy was **not run and is not claimed** in P314. No Lean toolchain was supplied in this pass, so formal gates were **not rerun and are not claimed**; the pass does not modify the existing Lean models.

## Known payers / hostile notes
1. Encrypted immutable sections currently authenticate at whole-section granularity. `copy_section_to` therefore requires materializing/decrypting an encrypted section rather than retaining the old raw streaming copy path. For very large encrypted sections this is a memory/latency payer and should be replaced by a bounded authenticated chunk layout rather than hidden behind a fallback.
2. WAL currently obtains a fresh random nonce for every new encrypted frame. This is safe and simple but the OS-random call can dominate sub-microsecond AEAD latency for tiny WAL records. A crash-safe session/counter nonce allocator (while retaining AES-GCM-SIV misuse resistance) is the next performance R&D target.
3. CFMD AE v1 currently exposes a raw external 256-bit master-key path. Password KDF/wrapped-master-key and general `KeyProvider` integration are deliberately not conflated with the primitive/storage pass.
4. Directory-storage encryption is not implemented; requesting it fails closed. Single-file is the normal product default established by P313.
5. Explicit external-freshness-aware low-level single-file APIs still need encryption configuration parity; ordinary product builder operation is covered, but this advanced boundary should be unified before declaring full encryption parity across every low-level entry point.

## Architectural conclusion
P314 establishes encryption as a codec/key-hierarchy concern inside the existing durability architecture rather than a parallel encrypted database implementation. AES-256-GCM-SIV is the v1 baseline, while the persisted algorithm identifier and typed protocol boundary leave room for future algorithms without runtime fallback or durability duplication.

## Next target — P315
Close the first production payers of CFMD AE v1 rather than expanding algorithm count:
1. design a bounded authenticated-chunk section format so encrypted section copy/read/compaction does not require whole-section materialization;
2. replace per-WAL-record OS-random calls with a crash-safe session/counter nonce namespace and benchmark end-to-end WAL cost;
3. introduce a clean external `KeyProvider` / wrapped database-master-key lifecycle (raw-key remains the minimal adapter, not the entire product model);
4. bring external-freshness-aware low-level single-file open/create paths onto the same encryption configuration;
5. add explicit per-key usage/epoch limits and rotation hooks before long-lived production deployment.

Only after those invariants are clean should an additional AEAD (for example a future pure-Rust runtime-dispatched AEGIS implementation) be considered.
