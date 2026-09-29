# IMPLEMENTATION REPORT — secure-memory R&D01

## Changed ownership

- Added `cfmd-secure-memory`: sole low-level secret-memory platform owner.
- `kernel-durability` consumes the owner; no durability-format change was introduced.
- `cfmd-runtime::EncryptionKey::from_bytes` is now fallible because hardened allocation is fail-closed.

## Implemented primitives

`SecureBytes<N>`:
- fixed-size protected byte storage;
- no `Clone`/`Copy`;
- closure-scoped access;
- redacted diagnostics.

`SecureBox<T>`:
- protected storage for secret-bearing algorithm objects;
- protected mapping is established before secret-bearing constructor execution via `try_new_with`;
- moved-from source representation scrubbed after placement;
- runs `T` destructor before full-page scrub/unmap.

Linux backend:
- anonymous guarded mapping;
- `mprotect` writable interior;
- `mlock`;
- `MADV_DONTDUMP`;
- volatile scrub + compiler fence;
- `munlock` + `munmap`.

## Crypto-path changes

`StorageEncryptionKey` -> `Arc<SecureBytes<32>>`.

Protected direct-write paths now include:
- OS-random DMK generation;
- wrapped-DMK decrypt output;
- wrapped-DMK plaintext staging before seal;
- HKDF derived keys.

Protected algorithm state now includes:
- HKDF derivation state;
- DMK-wrap AES-GCM-SIV context;
- persistent section AES-GCM-SIV context;
- persistent WAL AES-GCM-SIV context;
- persistent immutable-object AES-GCM-SIV context.

## Format compatibility

No storage envelope, nonce, AAD, wrapped-key header or durability-format version was changed. This is an in-memory ownership/security refactor only.
