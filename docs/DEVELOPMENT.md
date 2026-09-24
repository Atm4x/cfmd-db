# Development Guide

## RustRover / IDE import

Open the repository root containing `Cargo.toml`. The workspace is a standard Cargo workspace; no generated project files are required.

The repository pins Rust 1.98.1 in `rust-toolchain.toml`. Cargo is configured to use the committed `vendor/` source tree, so normal workspace operations do not require crates.io access.

Useful commands:

```bash
./scripts/verify-repository.sh
./scripts/ci-rust.sh
./scripts/stats.sh
```

If Lean 4.34.0 is installed:

```bash
./scripts/ci-formal.sh
```

## Where to start reading

1. `SPEC.md` — concise semantics and project boundaries.
2. `docs/spec/CFMD_CORE_SPEC.md` — full normative design.
3. `docs/architecture/ARCHITECTURE.md` — layer map and invariants.
4. `docs/architecture/CRATE_MAP.md` — workspace crate map.
5. `docs/status/PROJECT_STATUS.md` — current phase.
6. `docs/api/RUST_API_ROADMAP.md` — next implementation phase.

Do not start from `docs/reports/archive/` or `docs/history/` unless investigating provenance/regressions.

## Historical evidence policy

The repository deliberately retains old reports and raw evidence, but they are segregated from active source/docs. New pass reports belong in `docs/reports/current/` while active; once superseded, move them into the corresponding `docs/reports/archive/pass-NNN/` directory.

## Offline dependency policy

External crates are vendored. A dependency update should be atomic:

1. update the relevant manifest;
2. update `Cargo.lock`;
3. refresh `vendor/`;
4. run strict workspace gates;
5. update `THIRD_PARTY.md` and any security/dependency audit evidence.
