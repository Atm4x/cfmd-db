# CFMD Secure Memory R&D Ledger

Branch scope: anti-dump secret-memory infrastructure without a separate crypto process.

## Architecture decision

`cfmd-secure-memory` is infrastructure, not a database semantic kernel. It is therefore a `cfmd-*` crate rather than `kernel-secure-memory`. It is the only crate in this R&D branch allowed to contain the platform `unsafe` needed for `mmap`/`mlock`/`mprotect`/`madvise`; the rest of the workspace retains the existing unsafe-code prohibition.

## Security contract

Targeted:
- swap/page-out resistance for secret pages;
- exclusion from ordinary Linux core dumps;
- guard pages around secret storage;
- scrub-before-unlock/unmap;
- no ordinary `Copy`/`Clone` transport for secret bytes;
- persistent derived keys and AEAD key schedules outside ordinary heap/stack storage.

Not claimed:
- secrecy from root/kernel/hypervisor arbitrary live-memory inspection;
- Windows/macOS hardened backend yet;
- elimination of every transient secret-dependent register/stack value inside third-party cryptographic implementations.

## CLOSED — R&D01

- [x] Introduced `crates/cfmd-secure-memory` as the isolated platform-memory owner.
- [x] Linux mapping uses `PROT_NONE` outer guards, writable interior pages, `mlock`, and `MADV_DONTDUMP`.
- [x] Drop performs volatile full-page scrubbing before `munlock`/`munmap`.
- [x] Added `SecureBytes<N>` with closure-scoped byte access and redacted `Debug`.
- [x] Added `SecureBox<T>` for persistent secret-bearing algorithm state.
- [x] `SecureBox<T>` scrubs moved-from source storage after transfer into the protected mapping.
- [x] `/proc/self/smaps` regression verifies `lo` and `dd` on the actual data mapping.
- [x] `StorageEncryptionKey` now owns `Arc<SecureBytes<32>>`; clone shares the protected slot.
- [x] Random DMK generation writes directly into protected memory.
- [x] DMK unwrap decrypts directly into protected memory.
- [x] DMK wrapping plaintext staging uses protected memory.
- [x] HKDF outputs are written directly into protected memory.
- [x] HKDF state used for storage/wrap derivation is held in `SecureBox`.
- [x] Long-lived section/WAL/immutable-object `Aes256GcmSiv` contexts are `Arc<SecureBox<_>>`; codec clones no longer duplicate schedules into ordinary memory.
- [x] Raw `EncryptionKey::from_bytes` is fallible instead of silently degrading when hardened memory cannot be established.
- [x] Workspace `cargo check --workspace --all-targets --offline` passes.
- [x] `kernel-durability` 206-test library suite passes.
- [x] Product encrypted-open and four provider/rewrap regressions pass.

## OPEN

- [ ] Implement and verify a Windows backend (`VirtualAlloc`/`VirtualLock` plus a defensible crash-dump exclusion policy).
- [ ] Decide macOS/BSD support contract or explicit unsupported behavior.
- [ ] Replace the raw array-oriented provider handoff with a direct-fill/opaque-secret provider API so callers need not materialize `[u8; 32]` before entering CFMD.
- [ ] Audit third-party AEAD/KDF constructors for unavoidable transient copies and document the exact residual live-attack window.
- [ ] Add dump-harness tests that produce an actual core/minidump and search it for marked test secrets where the platform permits deterministic automation.
- [ ] Add RLIMIT/memory-lock exhaustion tests and product diagnostics.
- [ ] Decide whether hardened memory is mandatory product default or an explicit security profile after Windows behavior is closed.
- [ ] Benchmark real encrypted section/WAL throughput against PASS330 baseline; current microbench only establishes mapping-lifecycle vs steady-state cost.
- [ ] Review whether SHA-256 key-commitment transient state merits protected placement or is acceptable as short-lived constructor state.

## Performance note

R&D01 Linux microbench on the sandbox host:
- protected 32-byte slot create+drop: ~4–5.1 µs;
- ordinary boxed 32-byte create+drop: ~1.4–2.5 ns in the synthetic loop;
- protected steady-state byte access: ~0.5–0.8 ns/op;
- ordinary array access: ~0.28–0.30 ns/op.

Interpretation: protected mapping creation is intentionally a key/open/rotation lifecycle operation, never an AEAD hot-path operation. Once established, the protected mapping does not impose a meaningful database-throughput penalty by itself.

## CLOSED — R&D02

- [x] Full useful Rust 1.98.1 toolchain retained in the environment; only the distribution tar/staging is deleted.
- [x] Provider handoff is direct-fill: CFMD allocates hardened storage before invoking the provider.
- [x] Provider metadata is secret-free and separated from key material.
- [x] Provider `Ok` without destination initialization fails closed.
- [x] `SecureBox<T>` no longer implements `Deref`/`DerefMut`; algorithm state is closure-scoped.
- [x] Enabled `aes/zeroize`, `hmac/zeroize`, and `sha2/zeroize` through feature unification.
- [x] Added regression requiring `aes::Aes256: ZeroizeOnDrop`.
- [x] Hostile dependency audit identified the remaining HMAC constructor stack transient.
- [x] Full workspace check and Clippy `-D warnings` pass.
- [x] `kernel-durability` 207/207 library tests pass.

## OPEN after R&D02

- [ ] Implement and verify Windows hardened-memory backend and dump policy.
- [ ] Add actual core/minidump marker-search harness.
- [ ] Add deterministic locked-memory exhaustion/resource diagnostics tests.
- [ ] Benchmark real encrypted section/WAL throughput against PASS330.
- [ ] Resolve or explicitly accept/document `hmac::get_der_key` short-lived ordinary stack block.
- [ ] Audit constructor placement guarantees for third-party cipher/KDF values and decide whether stronger placement APIs are required.
- [ ] Decide mandatory default vs explicit hardened-memory security profile after Windows closure.
- [x] R&D02 feasibility: residual `hmac::get_der_key` ownership can be replaced without a vendor fork by a CFMD-owned fixed HKDF-SHA256 path using protected HMAC pads/SHA state while preserving RFC5869 output.
- [x] Final production-path inventory: provider-backed flow has no owned `[u8; 32]` key transport left inside durability; the public raw-key constructor is the intentional explicit escape hatch.
- [x] R&D02 release microbench confirms ~5.08 µs protected mapping lifecycle and ~0.413 ns steady-state protected read.

## CLOSED — R&D03

- [x] Replaced production `hkdf`/`hmac` path with a CFMD-owned fixed HKDF-SHA256 owner under `kernel-durability::storage_encryption`; `hkdf` remains dev-only for byte-compatibility tests.
- [x] PRK, HMAC pad, SHA-256 state and intermediate digest now share one protected workspace; no ordinary HMAC key block is created by the production KDF path.
- [x] Derived-key output slot is reused across section/WAL/immutable/commitment derivations, reducing secure mapping churn and peak locked memory.
- [x] Full key-commitment digest is protected; only the intended public 16-byte commitment leaves secure memory.
- [x] Added operation-specific secure-memory failures (`LockPages`, `ExcludeFromDump`, etc.) and a subprocess RLIMIT_MEMLOCK=0 regression that verifies fail-closed `LockPages` diagnostics.
- [x] Renamed dump capability to `native_crash_dump_excluded` to avoid claiming protection against arbitrary privileged dumpers.
- [x] Added Linux core-dump marker harness with an ordinary control marker; sandbox run explicitly SKIPs because kernel `core_pattern` is unavailable rather than producing a false PASS.
- [x] Added repeatable AEAD hot-path benchmark: protected cipher schedule is within measurement noise of ordinary placement.
- [x] Lifecycle benchmark identified and reduced secure codec init from ~156.8 us/op to ~44.0 us/op on this host; raw PASS330-like init is ~1.57 us/op.
- [x] Full `kernel-durability --lib`: 209 passed, 1 ignored manual benchmark; workspace Clippy `-D warnings` passes.

## OPEN after R&D03

- [ ] Implement and verify Windows backend: `VirtualAlloc`/guard pages + `VirtualLock` + WER exclusion, with semantics explicitly limited to native crash reporting.
- [ ] Run the core-dump marker harness on a Linux host/CI runner that exposes a directly discoverable kernel core file.
- [ ] Decide macOS/BSD support contract or explicit unsupported behavior.
- [ ] Audit/solve third-party AEAD constructor-placement transients: RustCrypto AES key expansion can create short-lived schedule values before the final `SecureBox` emplacement.
- [ ] Consider a multi-object `SecureArena` if many simultaneous databases make per-object page mappings or `RLIMIT_MEMLOCK` pressure material; current single-DB peak is small.
- [ ] Decide mandatory hardened-memory default vs explicit security profile after Windows closure.

## CLOSED — R&D04

- [x] Added `SecretHandle<T>` as cloneable opaque ownership for long-lived protected objects; cloning the handle never clones `T`.
- [x] Long-lived cipher ownership no longer leaks `Arc<SecureBox<Aes...>>` into storage code.
- [x] Added an explicit AEAD backend contract with static dispatch and backend-reported secret-state capabilities.
- [x] Moved RustCrypto AES-GCM-SIV implementation details (`Aes256GcmSiv`, nonce/tag conversion, in-place calls) into `storage_encryption::backend`.
- [x] DMK wrap/unwrap crypto is backend-owned through a protected ephemeral context; root storage encryption code no longer imports AES-GCM-SIV implementation types.
- [x] Corrected the security claim after source audit: current RustCrypto AES backend is `HardenedProcessMemory + ConstructorTransientsMayExist`, not strict in-place and not falsely certified as fully transient-zeroized.
- [x] Split encryption mode from algorithm: `StorageEncryption::{Direct, Wrapped}` now carries `StorageAeadAlgorithm` orthogonally, avoiding per-algorithm mode duplication.
- [x] Codec construction selects the backend before backend-specific context construction; future algorithms add a backend + one algorithm dispatch arm rather than an AES-shaped hidden path.
- [x] Single-file open/rewrap logic was split into semantic helpers instead of suppressing Clippy complexity warnings.
- [x] Full `kernel-durability --lib`: 211 passed, 2 ignored manual benchmarks.
- [x] `cfmd-secure-memory`: 5 unit + RLIMIT regression pass.
- [x] Workspace `cargo check --all-targets`, `cargo fmt --check`, and Clippy `-D warnings` pass.
- [x] Release backend-dispatch microbench shows no measurable penalty: ~0.995x direct at 4 KiB and ~0.981x at 64 KiB on this host (noise favors backend in this run).

## OPEN after R&D04

- [ ] Implement Windows hardened-memory backend before treating the branch as cross-platform production-ready.
- [ ] Run the Linux core-dump marker harness on a host/CI runner where kernel core files are directly collectible.
- [ ] Decide whether to accept RustCrypto constructor transients as the normal capability level or pursue an upstream strict in-place construction API/backend.
- [ ] Decide macOS/BSD support contract or explicit unsupported behavior.
- [ ] Consider a multi-object `SecureArena` only if profiling shows mapping/`RLIMIT_MEMLOCK` pressure across many simultaneously open databases.
- [ ] Decide product policy: hardened memory mandatory by default vs explicit security profile after Windows semantics are closed.

## Merge guidance after R&D04

R&D03 should not be merged as the architectural endpoint: R&D04 materially changes ownership, backend boundaries, capability reporting, and encryption configuration shape. R&D04 is a substantially better merge base for Linux-side development. For a cross-platform PASS334 product merge, the Windows backend remains the main blocker because current fail-closed secure-memory behavior would otherwise make encrypted open unsupported on Windows.
