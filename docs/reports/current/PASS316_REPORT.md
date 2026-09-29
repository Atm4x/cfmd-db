# PASS316 REPORT — Wrapped Database Master Key / crash-safe provider-key rotation

## Wall clock
- Start: 2026-09-29 00:16:04 UTC
- Useful boundary / functional freeze: 2026-09-29 00:36:04 UTC
- Hard boundary: 2026-09-29 00:40:04 UTC

## Goal
Close the next CFMD AE v1 productization payer without introducing a second encryption path: provider material must become a Key Encryption Key (KEK), each encrypted database must own a random Database Master Key (DMK), and provider/password/HSM rotation must be able to rewrap the DMK without rewriting generation/WAL ciphertext. The wrapped-key publication itself must have an explicit crash law.

## Delivered

### 1. Provider key is now a KEK, not the database data key
Provider-backed encrypted create generates a random 256-bit per-database DMK from the OS CSPRNG. The DMK remains the input to the existing AE v1 HKDF hierarchy for section/WAL keys.

Provider material is used only to derive a dedicated DMK-wrap key:

```text
provider KEK
    + database salt
        -> HKDF-SHA256("CFMD-AE-v1/dmk-wrap-key")
        -> AES-256-GCM-SIV wrap key
        -> wrapped random DMK

random DMK
        -> existing CFMD AE v1 section/WAL key hierarchy
```

Raw DMK/KEK bytes are never written to the database file.

### 2. Atomic provider-key snapshot
`cfmd-runtime` now exposes:
- `EncryptionKeyId` — fixed 128-bit provider key identity;
- `EncryptionProviderKey` — one atomic snapshot containing KEK bytes + key ID + provider epoch;
- `EncryptionKeyOperation::{Create, Open, Rewrap}`.

`EncryptionKeyProvider::provide_key(...)` returns that complete snapshot in one call. This deliberately avoids a split `key_id()/epoch()/key()` API where a dynamic provider could return mutually inconsistent values between calls.

All-zero key IDs and epoch zero fail closed at the product boundary.

### 3. Authenticated DMK wrapping
The wrapped-DMK AEAD associated data binds:

```text
CFMD-AE-v1/dmk-wrap
database salt
provider key ID
provider key epoch
database key epoch
wrapped-key publication sequence
```

Wrong KEK bytes and substitution of provider identity, provider epoch, database key epoch or publication sequence therefore fail AES-256-GCM-SIV authentication.

A dedicated regression verifies exact unwrap plus failure under wrong KEK / changed provider epoch / changed publication sequence.

### 4. Single-file format v4 dual key slots
Only the single-file **header** format advances to v4. Root and generation record formats remain v3 and therefore retain P315 compatibility.

Header v4 reserves two fixed 160-byte `CFKW` slots. Each slot carries:
- publication sequence;
- database key epoch;
- provider key epoch;
- provider key ID;
- random 96-bit wrap nonce;
- wrapped 256-bit DMK + 128-bit tag;
- independent SHA-256 physical checksum.

The static header digest remains separate from key-slot publication.

### 5. Crash-safe rewrap law
`SingleFileContainer::rewrap_database_master_key(...)` writes the inactive key slot and `sync_all()`s it before returning. Recovery parses both slots and chooses the highest valid publication sequence.

Therefore:

```text
old slot valid
    -> begin write of inactive new slot
        -> crash/torn write: invalid new slot ignored, old slot remains authority
        -> complete + durable new slot: higher sequence is authority
```

A **valid** newer slot is never error-fallback-routed to the older slot. Product regression confirms the old provider can no longer open after successful rotation while the new provider can.

The two slots are a crash-consistency mechanism, not a claim that an offline attacker cannot restore/corrupt an older complete header. Provider revocation/epoch policy and external freshness remain the anti-rollback authorities.

### 6. Product rewrap DX
An opened product database now exposes:

```rust
let next_epoch = db.rewrap_encryption(
    Encryption::aes256_gcm_siv_with_provider(next_provider)
)?;
```

The runtime holds the already-unlocked DMK, resolves the new provider KEK with `EncryptionKeyOperation::Rewrap`, publishes the next wrapped-key slot, and returns the new database key epoch.

Regression evidence compares bytes from the single-file data region (`DATA_OFFSET..EOF`) before and after rewrap and proves they are byte-for-byte identical. Rewrap changes key-envelope metadata only; checkpoint/generation/WAL ciphertext is not re-encrypted.

### 7. Explicit compatibility law
P314/P315 format-v3 direct-key files remain readable. In particular, a legacy provider-backed v3 database (where provider material was historically the effective master key) can still be opened through the provider path; new provider-backed creates always use v4 wrapped-DMK semantics.

Raw `Encryption::aes256_gcm_siv(key)` remains a minimal direct-key adapter. It is intentionally not pretending to have wrapped-key rotation semantics.

### 8. Hostile correction found during P316
Initial P316 implementation reused the old global `FORMAT_VERSION` constant for the new header version. In `single_file.rs` that constant also owns generation/root physical record versions; blindly changing it from 3 to 4 would have silently broken real P315 root/generation compatibility while a synthetic header-only test could still pass.

Hostile review caught this before freeze. The final implementation separates:

```text
FORMAT_VERSION               = 3   // root/generation physical records
LEGACY_HEADER_FORMAT_VERSION = 3
HEADER_FORMAT_VERSION        = 4   // encryption/key-management header only
```

A dedicated v3 compatibility regression now verifies the intended boundary.

## Verification completed before freeze
- `cargo test -p kernel-durability --offline --lib`: **181/181 PASS**
- `cargo test -p cfmd-runtime --offline`: **32/32 PASS**
- provider-backed random-DMK create/open: **PASS**
- authenticated wrap metadata substitution rejection: **PASS**
- successful KEK rewrap with unchanged data-region ciphertext: **PASS**
- old-provider rejection after valid new slot publication: **PASS**
- torn/incomplete new wrapped-key slot recovers prior valid slot: **PASS**
- legacy header-v3/provider compatibility: **PASS**
- `RUSTFLAGS=-D warnings cargo check -p kernel-durability -p kernel-plan -p cfmd-runtime -p cfmd-host --offline`: **PASS**
- `cargo fmt --all -- --check` with supplied Rust 1.98.1 rustfmt: **PASS**
- repository verifier (required files + Rust include targets + vendor checksums + repository manifest): **PASS**
- repository manifest: **3394 files**

Lean was not required in P316: the existing Lean publication model/refinement binders were not modified, and P316 adds a header-local cryptographic key-envelope protocol without changing the P306-P315 root/generation/WAL publication law. The supplied Lean 4.34.0 split archives were left untouched.

## Remaining payers
1. **External key-epoch / revocation authority**: dual slots close torn-write recovery, but minimum accepted provider/database key epoch should be bindable to an external provider/freshness policy so an obsolete complete header cannot be intentionally restored and accepted with a still-available old KEK.
2. **Password adapter**: add Argon2id (or another deliberately selected password KDF) as a KEK provider adapter; password changes then use the same DMK rewrap operation rather than data re-encryption.
3. **Provider/HSM adapters**: formalize provider lookup/open semantics for DPAPI/TPM/Keychain/KMS/HSM without leaking platform-specific concerns into `kernel-durability`.
4. **Fully streaming encrypted publication**: encrypted section reads/copies are bounded, but initial generation publication still stages encrypted stored-section bytes before writing the table/digest.
5. **Directory encryption parity**: still fail-closed and should eventually reuse the same AE codec/key hierarchy, not create a parallel encryption implementation.
6. **Key usage/rotation policy**: database key epoch now exists physically; automatic AEAD usage limits and DMK rotation/re-encryption policy remain separate from cheap KEK rewrap.

## Architectural conclusion
P316 separates long-lived data confidentiality from credentials/key-provider lifecycle. The random database DMK is stable across provider/password rotation; external provider material is a replaceable KEK. Wrapped-key publication has its own alternating-slot crash law and does not alter CFMD's root/generation/WAL authority mathematics. No crypto-algorithm routing or plaintext fallback was added.

## Next target — P317
Perform a hostile key-management pass around **anti-rollback and provider authority**, then close the next physical payer:

1. define a minimum accepted provider/database-key epoch contract and bind it to external freshness/provider policy where available;
2. prove/open-test that valid-new-slot authentication/provider failures never fall back to an older slot;
3. decide whether obsolete key slots should be retired after externally acknowledged rotation or deliberately retained only as crash evidence;
4. add a clean password→KEK adapter only after selecting and vendoring the password KDF intentionally;
5. begin bounded/streaming encrypted generation publication if the authority proof remains unchanged.

Do not add a second AEAD in P317 unless it materially helps this key-authority work.
