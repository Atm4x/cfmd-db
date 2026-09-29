# R&D04 — Opaque Secret Handles and Crypto Backend Boundary

## Goal

Stabilize the secure-memory architecture before mainline merge: make persistent crypto state opaque, separate algorithm implementation from storage semantics, make backend security guarantees explicit, and remove AES-specific mode duplication without adding hot-path virtual dispatch.

## Result

`cfmd-secure-memory` now provides `SecretHandle<T>` for shared long-lived protected state. A handle clone shares one hardened object and cannot clone/extract `T` through the API.

`kernel-durability::storage_encryption` now owns a private static-dispatch backend layer. RustCrypto AES-GCM-SIV implementation details and DMK wrap operations live behind that layer. Storage code sees `BackendState`, `StorageAeadAlgorithm`, and an explicit capability record.

The RustCrypto source audit changed one prior assumption: AES round-key backend values are `Copy` during construction. Therefore the current backend is honestly reported as:

- persistent state: `HardenedProcessMemory`;
- initialization: `ConstructorTransientsMayExist`;
- strict in-place: false.

No vendor fork was introduced.

Encryption configuration is now orthogonal:

```text
StorageEncryption::Direct  { algorithm, key }
StorageEncryption::Wrapped { algorithm, provider metadata... }
```

rather than one pair of enum variants per algorithm. Runtime convenience constructors remain possible without shaping the durability kernel around AES.

## Performance

Release backend dispatch benchmark:

- 4 KiB: direct 2491.6 ns/op, backend 2478.0 ns/op, ratio 0.9945;
- 64 KiB: direct 25805.7 ns/op, backend 25302.9 ns/op, ratio 0.9805.

The difference is benchmark noise; static enum/backend dispatch does not show a measurable throughput cost.

## Validation

- `cfmd-secure-memory`: 5 unit tests + RLIMIT_MEMLOCK subprocess regression pass.
- `kernel-durability --lib`: 211 passed, 2 ignored manual benchmarks.
- full workspace check: pass.
- full workspace Clippy `-D warnings`: pass.
- `cargo fmt --check`: pass.
- single-file encrypted/rewrap/fault scenarios remain green.

## Merge recommendation

Do not merge R&D03 as the final shape; R&D04 materially supersedes it. R&D04 is suitable as the Linux architectural base. For PASS334 as a cross-platform product baseline, finish the Windows backend first; otherwise encrypted CFMD intentionally fails closed on Windows.
