# Implementation Report — Secure Memory R&D02

## Changed architecture

### Provider boundary

`EncryptionKeyProvider` no longer returns an object containing secret key material. CFMD first allocates `StorageEncryptionKey` in hardened memory, then calls the provider with `EncryptionKeyDestination`.

`EncryptionKeyDestination` offers:
- `write(&[u8; 32])` for compatibility/convenience;
- `fill_with(...)` for APIs capable of writing directly into a caller-provided buffer.

A successful provider call is accepted only if the destination was initialized. Provider identity/epoch/floor are represented by `EncryptionProviderKeyMetadata`, which contains no secret.

### Secret object access

`SecureBox<T>` now exposes only `with_secret` / `with_secret_mut`. Removing `Deref`/`DerefMut` prevents algorithm methods and `Clone` from being reached through autoderef and accidentally exporting protected state.

### Dependency-state scrubbing

`kernel-durability` enables:
- `aes/zeroize`;
- `hmac/zeroize`;
- `sha2/zeroize`.

This hardens per-message AES schedules and HMAC/SHA states on Drop. AES zeroizing Drop is compile-time/regression tested.

## Residual risk found by hostile audit

The current HMAC dependency's `get_der_key` builds a derived HMAC key block as an ordinary local stack object and does not explicitly zeroize that local before returning. This is short-lived and does not invalidate protected persistent state, but it remains the clearest transient software residue to address or formally accept/document.

## Compatibility

No storage-format or cryptographic-envelope change. The R&D provider API is intentionally source-breaking because the previous API forced key ownership into the provider-return object and conflicted with direct-fill secure-memory semantics.

## Concrete next step

Source audit confirms the HMAC-constructor residual can be removed without patching vendored crates. A CFMD-owned fixed HKDF-SHA256 implementation can keep the 64-byte HMAC key pad in `SecureBytes<64>`, keep SHA-256 state in `SecureBox<Sha256>`, use reset-capable finalization, immediately zeroize digest outputs, and preserve RFC5869 byte compatibility. This is preferred over a vendor fork and is the next R&D target.
