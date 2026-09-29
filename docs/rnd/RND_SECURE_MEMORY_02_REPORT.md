# RND SECURE MEMORY 02 REPORT

Start: 2026-09-29 09:38:27 UTC  
Useful cutoff: 2026-09-29 09:58:27 UTC  
Hard cutoff: 2026-09-29 10:02:27 UTC

## Result

R&D02 closes the raw provider handoff and tightens the protected-state API. CFMD now allocates hardened key storage before invoking an external `EncryptionKeyProvider`; the provider receives an `EncryptionKeyDestination` and writes directly into that slot. Provider metadata is returned separately and no longer carries secret material. Returning success without initializing the destination fails closed.

`SecureBox<T>` no longer implements `Deref`/`DerefMut`. Secret-bearing algorithm state is accessible only through closure-scoped access, preventing safe-Rust autoderef from invoking methods such as `T::clone()` and materializing an AES/HKDF state outside hardened memory.

The vendored crypto audit also found that `aes-gcm-siv` zeroizes its explicit per-message key buffers, but the temporary AES schedule only receives guaranteed Drop scrubbing when `aes/zeroize` is enabled. R&D02 enables `aes/zeroize`, `hmac/zeroize`, and `sha2/zeroize` through feature unification. A regression asserts `aes::Aes256: ZeroizeOnDrop`.

No on-disk format, nonce semantics, DMK wrapping format, or selected AEAD algorithm changed.

## Architecture state

Closed on Linux:
- hardened guarded/locked/dump-excluded secret mappings;
- scrub-before-unmap;
- DMK generation/unwrap and HKDF output direct into protected slots;
- persistent HKDF and AES-GCM-SIV state protected;
- provider direct-fill into preallocated protected memory;
- provider success without key initialization rejected;
- protected object API cannot autoderef/clone secret state outward;
- AES schedule and HMAC/SHA state Drop hardening enabled where supported by dependencies.

Still open:
- Windows backend and defensible Windows dump policy;
- actual core/minidump harness that searches for marker secrets;
- deterministic memory-lock exhaustion diagnostics/tests;
- real encrypted section/WAL throughput comparison against PASS330;
- residual third-party constructor transient: `hmac::get_der_key` creates a short-lived ordinary stack block without explicit zeroization;
- final decision on mandatory-vs-profile hardened-memory product policy.

Rough engineering proximity: Linux single-process foundation is ~75–80% of the planned production-grade workstreams. Cross-platform completion is materially lower until Windows is closed.

## Validation

- Full useful Rust toolchain retained: rustc/cargo/rustfmt/clippy/rust-analyzer 1.98.1; only the supplied 194 MB distribution tar and extraction staging were deleted.
- `cargo fmt --all` clean.
- `cargo check --workspace --all-targets --offline` passes.
- `cargo clippy --workspace --all-targets --offline -- -D warnings` passes.
- `cfmd-secure-memory`: 4/4 tests pass.
- `kernel-durability --lib`: 207/207 tests pass.
- provider direct-fill and fail-closed destination tests pass.
- encryption provider regressions pass.

## Next R&D target resolved

The remaining HKDF/HMAC constructor residue does not require a vendor fork. The next branch can replace the current generic HKDF constructor with a narrow CFMD HKDF-SHA256 owner: a protected 64-byte HMAC key block (`SecureBytes<64>`), protected `Sha256` state (`SecureBox<Sha256>`), `finalize_reset`, and immediately-zeroized 32-byte digest outputs copied into protected slots. `sha2/zeroize` plus digest/block-buffer zeroization already provide Drop scrubbing for SHA state/buffers. This keeps RFC5869 output compatibility while eliminating `hmac::get_der_key` as the key-block owner.

## Final hostile inventory

A focused production-path grep after the refactor finds no remaining owned `[u8; 32]` key transport inside `kernel-durability::storage_encryption`. The remaining 32-byte references there are database salt references. At the public runtime boundary, `EncryptionKey::from_bytes([u8; 32])` remains intentionally as the explicit raw-key escape hatch, while provider-backed operation uses the direct-fill destination.

R&D02 release microbench: protected slot create+drop ~5.08 µs; protected steady-state read ~0.413 ns/op; ordinary array read ~0.295 ns/op. The cost remains concentrated in key lifecycle rather than the crypto hot path.
