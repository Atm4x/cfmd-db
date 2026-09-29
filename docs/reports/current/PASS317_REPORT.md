# PASS317 REPORT — Pre-release format cleanup / provider key-authority floor

## Wall clock
- Start: 2026-09-29 00:42:25 UTC
- Functional freeze: 2026-09-29 01:02:25 UTC
- Useful boundary: 2026-09-29 01:02:25 UTC
- Hard boundary: 2026-09-29 01:06:25 UTC

## Goal
Correct the P316 assumption that internal R&D layouts need release-style compatibility versioning, then close the first honest anti-rollback payer for wrapped database keys. CFMD has not shipped an on-disk compatibility contract, so pass-to-pass formats should stay one current design rather than accrete legacy routing. At the same time, crash-safe dual key slots must gain an external authority that can reject a complete but obsolete header, not merely torn writes.

## Delivered

### 1. Removed artificial P316 header-version split
P316 introduced `LEGACY_HEADER_FORMAT_VERSION = 3` and `HEADER_FORMAT_VERSION = 4` only to preserve P314/P315 internal snapshots. That is not a product requirement before the first released format.

P317 removes both constants and the compatibility branch. Header, root and generation records now use the single current `FORMAT_VERSION` again. The version marker remains a fail-closed physical-format discriminator for future released evolution; it is not used as a reason to carry R&D-pass migration code.

The synthetic `legacy_v3_direct_key_store_remains_readable_through_provider_compatibility` regression and the provider-as-direct-key compatibility path were deleted.

### 2. Removed pre-release whole-section encryption compatibility routing
P315 still kept the P314 whole-section encrypted section envelope as a read compatibility path. No released database depends on it, so P317 removes that branch as well.

The current encrypted-section law is now singular:

```text
encrypted section
    -> CFSC bounded chunk layout
    -> 64 KiB authenticated chunks
    -> unknown/non-CFSC encrypted layout fails closed
```

There is no whole-section fallback and no pass-number-specific storage routing.

### 3. External minimum database-key epoch
`EncryptionProviderKey` now carries a non-zero `minimum_database_key_epoch` authority. The default constructor admits epoch 1; security-sensitive providers can raise the floor through:

```rust
EncryptionProviderKey::new(key, key_id, provider_epoch)
    .with_minimum_database_key_epoch(epoch)
```

This floor comes from the provider outside the database file. It is therefore not attacker-controlled metadata restored together with an old `.cfmd` image.

Wrapped-key open selects the highest valid in-file slot exactly as before, then fails closed before DMK unwrap when:

```text
slot.database_key_epoch < provider.minimum_database_key_epoch
```

The error is explicit: `wrapped database key was rolled back below provider authority floor`.

### 4. Complete-header rollback regression
A new low-level regression:
1. creates a wrapped-key database at database-key epoch 1;
2. saves the complete valid header page;
3. rewraps the same DMK to epoch 2;
4. restores the old complete epoch-1 header, not a torn/corrupt header;
5. opens with the same still-valid provider KEK but external minimum database-key epoch 2;
6. verifies fail-closed rejection.

A product-level `cfmd-runtime` regression performs the same scenario through `EncryptionKeyProvider` and `Database::rewrap_encryption(...)`.

This proves the distinction between the two authorities:

```text
dual CFKW slots
    -> crash/torn-write recovery

external provider epoch floor
    -> complete-header rollback fence
```

### 5. Rotation handoff contract
`Database::rewrap_encryption(...)` still returns the newly durable database-key epoch. The intended external-provider sequence is therefore:

```text
publish + sync wrapped slot N
    -> rewrap_encryption returns N
    -> provider/KMS durably raises minimum accepted DB epoch to N
```

The provider must not raise the floor before successful rewrap publication; otherwise a crash could intentionally make the still-authoritative previous slot unopenable. P317 provides the floor and verifies it, but does not pretend remote provider state and local fsync are one atomic transaction. An acknowledged remote/KMS floor-advance protocol remains a separate payer.

Create fails if the provider floor would reject initial database-key epoch 1. Rewrap fails if the requested floor exceeds the epoch being published. Floor zero is invalid.

### 6. Strict-Clippy hostile cleanup
The first strict all-target Clippy run surfaced several ownership/representation issues around the newly enlarged encryption state. P317 fixed them rather than suppressing them:

- `SingleFileEncryptionHeader` is passed by reference instead of copying a ~280-byte structure;
- `StorageEncryption` derives `Default` directly;
- the large `SingleFileDurabilityBackend` enum arm is boxed;
- streaming-checkpoint `shadow_wal` is boxed instead of inflating every `StreamingCheckpointPhysical` value;
- storage-option and rewrap helpers borrow non-consumed configuration instead of cloning/moving it through internal layers;
- WAL recovery uses the simpler single-pattern branch;
- one test-only narrowing cast became checked conversion.

The only Clippy allow retained is the existing-style test-local `too_many_lines` allowance for the long provider-rotation E2E scenario; production code passes strict all-target Clippy without new suppressions.

## Verification
- `kernel-durability`: **181/181 PASS** plus multiprocess suites PASS
- `cfmd-runtime`: **33/33 PASS**
- `cfmd-host`: **8/8 PASS**
- complete-header rollback below provider floor: low-level PASS
- complete-header rollback below provider floor: product/provider PASS
- strict `cargo clippy -p kernel-durability -p kernel-plan -p cfmd-runtime -p cfmd-host --all-targets -- -D warnings`: **PASS**
- strict `RUSTFLAGS=-D warnings cargo check` for the same production crates: **PASS**
- strict `RUSTDOCFLAGS=-D warnings cargo doc --no-deps` for the same production crates: **PASS**
- `cargo fmt --all -- --check`: **PASS**
- repository verifier: **PASS**
- repository manifest: **3395 files**
- Rust: supplied **1.98.1** toolchain

Lean is not required for P317. Root/generation/WAL publication authority and its crash points are unchanged; P317 removes a header compatibility branch and adds an external admission floor before DMK unwrap. The supplied Lean 4.34.0 archives were left untouched.

## Architectural conclusion
Before the first released CFMD format, there is one current on-disk design. Internal pass history is evidence, not a migration burden. `FORMAT_VERSION` remains only as a fail-closed physical schema identifier; P317 no longer increments or routes on internal pass layouts.

Wrapped-key rollback now also has a clean authority split: local dual slots answer "which completely written key envelope survived the crash?", while the provider floor answers "how old a cryptographically valid database-key epoch am I still willing to trust?" Neither substitutes for the other.

## Next target — P318
1. Design an acknowledged provider/KMS floor-advance protocol so remote anti-rollback authority can be advanced after local key-slot durability without hand-wavy ordering.
2. Decide whether externally acknowledged old `CFKW` slots should be retired/zeroed or deliberately retained only as crash evidence.
3. Attack fully streaming encrypted generation publication; current read/copy is bounded but generation creation still stages stored encrypted-section bytes.
4. Then close directory-encryption parity using the same AE v1 codec/key hierarchy rather than a second implementation.
5. Password→KEK/Argon2id remains product adapter work after an explicit dependency/security review; do not mix it into the durability authority protocol.
