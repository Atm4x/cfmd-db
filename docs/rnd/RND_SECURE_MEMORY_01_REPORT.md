# RND SECURE MEMORY 01 REPORT

Start: 2026-09-29 09:13:44 UTC  
Useful cutoff: 2026-09-29 09:33:44 UTC  
Hard cutoff: 2026-09-29 09:37:44 UTC

## Result

The R&D branch now has a real single-process hardened-memory foundation rather than key obfuscation.

A new `cfmd-secure-memory` infrastructure crate owns the platform memory boundary. On Linux it allocates guarded anonymous mappings, locks the data pages, marks them `MADV_DONTDUMP`, and scrubs the full writable mapping before unlock/unmap. A `/proc/self/smaps` regression verifies the kernel reports both `lo` and `dd` for the protected data VMA.

`kernel-durability` is wired into that owner:
- external/raw storage keys become shared protected `SecureBytes<32>`;
- random DMK, unwrap plaintext, wrap plaintext staging and HKDF outputs avoid ordinary persistent byte arrays;
- HKDF state and long-lived AES-GCM-SIV contexts live in `SecureBox<T>`;
- `StorageAeadCodec::clone()` shares protected cipher contexts via `Arc` rather than cloning AES schedules;
- failure to establish hardened memory propagates instead of silently falling back to ordinary heap memory.

The new owner is intentionally named `cfmd-secure-memory`, not `kernel-secure-memory`: page locking and dump policy are host/platform infrastructure, not semantic database authority.

## Validation

- Rust 1.98.1 compiler and cargo were extracted from the supplied archive; a standalone probe compiled and executed successfully.
- Supplied 194 MB Rust source tar was deleted immediately after installation.
- `cfmd-secure-memory`: 4/4 unit tests pass.
- Linux `/proc/self/smaps`: protected mapping has `lo` + `dd`.
- `kernel-durability --lib`: 206/206 tests pass.
- `cfmd-runtime` encrypted single-file regression passes.
- Four provider/key-epoch/rewrap regressions pass.
- `cargo check --workspace --all-targets --offline` passes.

`rustfmt` and `clippy` were not present because only compiler/std/cargo were extracted before deleting the supplied tar. No network/toolchain re-download was performed.

## Performance

Synthetic R&D measurement shows ~4–5.1 µs create+drop for a protected 32-byte mapping and ~0.5–0.8 ns steady-state protected read versus ~0.28–0.30 ns ordinary-array read. The expensive operation is page lifecycle, which occurs at open/key setup/rotation; the database AEAD hot path uses already-established protected contexts.

## Remaining security boundary

This branch materially protects ordinary core/crash dumps, swap/page-out and secret residue. It does not claim to defeat an attacker with kernel/root/hypervisor live-memory authority. The largest remaining software integration issue is the raw provider API: a caller can still own/copy its `[u8; 32]` before passing it to CFMD. A direct-fill or opaque-secret provider boundary is the next clean R&D target.
