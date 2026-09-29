# PASS327 REPORT — Authenticated immutable replication-authority objects

## Wall clock
- Start: 2026-09-29 07:06:09 UTC
- Functional freeze: 07:25:12 UTC
- Useful boundary: 07:26:09 UTC
- Hard boundary: 07:30:09 UTC

## Goal
Close the release-blocking security flaw found by P326 before reactivating its otherwise successful linked-segment physical architecture. External immutable replication-authority payloads must remain under CFMD AE v1 for encrypted databases without making `ReplicationAuthoritySegmentId` depend on nonce, generation, or physical location, and without reintroducing payload-sized buffering or a plaintext fallback.

## Delivered

### 1. Dedicated AE v1 immutable-object domain
`StorageEncryptionDomain` now contains `ImmutableObject`. `StorageAeadCodec` derives its key independently with HKDF-SHA256 alongside the existing section and WAL keys:

```text
DMK + database salt
    -> section key
    -> WAL key
    -> immutable-object key
```

A regression proves that identical nonce/context/plaintext produce different envelopes in all three domains. This is protocol domain separation, not primitive routing: AES-256-GCM-SIV remains the single AE v1 production primitive.

### 2. `CFAO` authenticated immutable-object envelope
Added the first external-object codec under the P325 segment owner. The fixed object header binds:

```text
object kind = replication authority segment
plaintext ReplicationAuthoritySegmentId
parent ReplicationAuthoritySegmentId (or zero)
canonical plaintext length
fixed chunk size
chunk count
```

For encrypted stores the exact complete header participates in every chunk's AAD together with chunk index and chunk plaintext length. Header/identity/parent/length substitution therefore fails authentication rather than becoming an alternate parse.

### 3. Bounded 64 KiB encryption and recovery
`CFAS` plaintext is streamed through the object codec in fixed 64 KiB chunks. Each encrypted chunk is an independent AE v1 envelope. The writer never constructs whole-segment ciphertext, and the reader authenticates one complete chunk before releasing bytes from it.

A multi-chunk regression builds a segment larger than two chunks, verifies that the largest emitted ciphertext allocation is bounded by one chunk envelope, validates the complete segment through the streaming reader, and proves that swapping two otherwise valid encrypted chunk envelopes is rejected by chunk-index AAD.

### 4. Plaintext identity remains canonical
The P325 identity law is unchanged:

```text
SegmentId = H(domain || parent || delta length || frame count || exact canonical frames)
```

`CFAO` nonce and ciphertext are physical representation only. Tests encrypt the same segment under independent nonce namespaces and prove the object bytes differ while the embedded/plaintext `SegmentId` is unchanged.

### 5. Relocation needs no decrypt/re-encrypt
Object AAD intentionally excludes generation number and physical offset. An encrypted object is therefore relocatable by exact authenticated byte copy. A regression prepends unrelated bytes, moves the object to a different offset, and successfully verifies/replays it under the same segment identity.

This establishes the intended P328 compaction law:

```text
reachable CFAO bytes
    -> copy exact bytes
    -> rebuild physical locator positions
    -> publish relocated root
```

not `decrypt -> encrypt` during compaction.

### 6. Pre-publication plan validation
Before the first `CFAO` byte is emitted, the object writer now revalidates that the supplied frame set still matches the frozen segment plan. A hostile regression mutates the frame list after planning and proves rejection occurs with exactly zero emitted physical bytes. This avoids creating avoidable orphan object headers from a stale publication plan.

### 7. Encryption mode is fail-closed
Plaintext and encrypted objects use one semantic grammar but are not fallback routes. Opening a plaintext object as encrypted or an encrypted object without the database crypto context is rejected. Ciphertext tamper and AAD/header tamper are both rejected.

### 8. Existing two-pass authority mutation law is preserved
The object reader feeds the P325 segment verifier. Recovery still performs a complete segment verification pass before the second replay pass mutates `ReplicationAuthorityJournal`. The object layer does not introduce a second replication evaluator.

## Hostile review / scope decision

P326's physical candidate remains rolled back. P327 deliberately did **not** rush the linked locator/root integration back into `SingleFileContainer` after adding encryption. The maintained P323 generation-contained archive therefore remains current product recovery authority, so this pass cannot reintroduce the plaintext-authority regression P326 rejected.

No `FORMAT_VERSION` bump was made. CFMD has not released an on-disk compatibility boundary, and more importantly `CFAO` is not yet active product root authority in this pass. Raising the current single-file format marker merely because an inactive next-format object codec exists would recreate the pre-release pass-version churn removed in P317.

The remaining payer is physical activation, not cryptographic design: ordinary checkpoint still copies the historical P323 replication archive until the linked locator/root path returns on top of `CFAO`.

## Verification
- Rust 1.98.1 minimal toolchain (`rustc + rust-std + cargo + clippy/rustfmt`) installed from the supplied bundle; standalone smoke program compiled and ran; supplied tar archive deleted afterward.
- `kernel-durability --lib` / primary all-target unit suite: **200/200 PASS**.
- `kernel-durability --all-targets`: **PASS**, including multiprocess suites.
- targeted `object_codec`: **7/7 PASS**, including every single-bit mutation of the 88-byte encrypted object header.
- `kernel-durability` strict Clippy `--all-targets -- -D warnings`: **PASS**.
- workspace `cargo check --all-targets` with `RUSTFLAGS=-D warnings`: **PASS**.
- workspace strict Clippy `--all-targets -- -D warnings`: **PASS**.
- `kernel-durability` rustdoc with `RUSTDOCFLAGS=-D warnings`: **PASS**.
- `cargo fmt --all -- --check`: **PASS**.
- repository manifest/layout verifier: **PASS** after final manifest regeneration.

## Next target — P328
Reactivate the physically proven P326 architecture, but only through P327 `CFAO` objects:

1. append each frozen live replication delta as one immutable `CFAO(CFAS)` object;
2. append persistent O(1)-per-delta linked locator metadata rather than rewriting full `CFAI`;
3. bind the current segment ID + locator position/digest into authoritative root/generation state;
4. recover locator chain -> relocatable in-memory index -> P327 object authentication -> P325 two-pass segment replay;
5. make compaction traverse only the reachable locator/object closure, copy exact authenticated object bytes, rebuild physical locator locations, then publish the relocated root;
6. run crash/torn/tamper/relocation regressions for both plaintext and AES-256-GCM-SIV stores;
7. only after those pass, delete P323 historical replication-archive copying from production.
