# RND SECURE MEMORY 03 REPORT

Start: 2026-09-29 10:00:15 UTC  
Useful cutoff: 2026-09-29 10:20:15 UTC  
Hard cutoff: 2026-09-29 10:24:15 UTC

## Result

R&D03 closes the remaining CFMD-owned HKDF/HMAC transient and adds operational verification around locked-memory failure, crash-dump policy and performance.

The production storage-encryption KDF no longer constructs RustCrypto `Hmac/Hkdf` state. A private fixed HKDF-SHA256 implementation owns one protected workspace containing PRK, HMAC pad, SHA-256 state and intermediate digest. Expansion writes into a protected output slot. The same derived-key slot is reused sequentially for section, WAL, immutable-object and commitment domains. `hkdf` is now dev-only and is used solely as a byte-compatibility oracle.

The full SHA-256 key-commitment digest is also kept protected; only its intended public 16-byte prefix leaves secure memory.

## Diagnostics and dump contract

`SecureMemoryError` now reports the failed operation. A subprocess regression lowers `RLIMIT_MEMLOCK` to zero and confirms allocation fails specifically at `LockPages`.

The protection capability was renamed from the overly broad `dump_excluded` to `native_crash_dump_excluded`. Linux maps this to `MADV_DONTDUMP`. Windows R&D found that `VirtualLock` covers pagefile resistance while `WerRegisterExcludedMemoryBlock` is specifically a Windows Error Reporting exclusion; arbitrary third-party minidump policy is a separate boundary.

A real core-dump marker harness was added. It requires a control marker in the generated core before accepting absence of the secure marker. The current sandbox has no exposed `/proc/sys/kernel/core_pattern`, so the harness returns SKIP (77); this is intentionally not counted as a security PASS.

## Performance

Release AEAD schedule placement benchmark, five runs:
- 4 KiB secure/raw median ratio: ~1.010x; mean ~1.001x.
- 64 KiB secure/raw median ratio: ~0.992x; mean ~0.990x.

No systematic hot-path throughput regression is measurable from protected placement itself.

Lifecycle initialization initially measured ~156.8 us/op versus ~1.61 us/op for a PASS330-like raw HKDF/AES path. Consolidating HMAC temporaries into one protected workspace reduced this to ~62.1 us/op; reusing one protected derived-key slot reduced it again to ~44.0 us/op versus ~1.57 us/op raw. The remaining cost is page mapping/locking for lifecycle objects, not AEAD throughput.

## Validation

- `cargo fmt --all`: clean.
- `cargo clippy --workspace --all-targets --offline -- -D warnings`: pass.
- `cfmd-secure-memory`: 4 unit + 1 RLIMIT integration test pass.
- `kernel-durability --lib`: 209 passed; 1 ignored manual lifecycle benchmark.
- fixed HKDF-SHA256 output matches RustCrypto HKDF reference.
- storage encryption regressions pass.
- Rust 1.98.1 full toolchain remains installed; no toolchain cleanup was performed.

## Remaining security boundary

The main Linux software-side residual is now third-party cipher construction. `Aes256GcmSiv::new` ultimately expands the AES key before the returned object is emplaced into `SecureBox`; the final moved-from object is scrubbed, and AES zeroizing Drop support is enabled, but Rust does not provide a general guarantee that every compiler/backend constructor temporary was created directly in the protected mapping. Eliminating that residual requires an in-place crypto construction API or a controlled backend implementation, not additional wrapping around the current constructor.
