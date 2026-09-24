# CFMD

CFMD is an experimental embedded database kernel built around a constructive finite-model view of database state. The repository currently contains the production kernel, durability/replication/security layers, executable law tests, and Lean mechanizations that bind selected architectural claims to the Rust implementation.

## Repository status

The historical kernel backlog (#1–#22) is closed for the declared supported scope. The final durability item (#13) is certified for the recorded QEMU/TCG + Linux/ext4 profile; additional bare-metal or filesystem profiles require separate certification evidence.

CFMD is now transitioning from kernel R&D to a stable user-facing Rust library surface. The internal crates are **not** the intended public API.

See:

- [`SPEC.md`](SPEC.md) — concise project specification and invariants;
- [`docs/spec/CFMD_CORE_SPEC.md`](docs/spec/CFMD_CORE_SPEC.md) — full normative core specification;
- [`docs/architecture/ARCHITECTURE.md`](docs/architecture/ARCHITECTURE.md) — current repository architecture;
- [`docs/status/PROJECT_STATUS.md`](docs/status/PROJECT_STATUS.md) — current state and next phase;
- [`docs/status/HISTORICAL_PROBLEMS_LEDGER.md`](docs/status/HISTORICAL_PROBLEMS_LEDGER.md) — closed historical ledger;
- [`docs/api/RUST_API_ROADMAP.md`](docs/api/RUST_API_ROADMAP.md) — next public-Rust-API phase;
- [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md) — RustRover/offline development guide;
- [`docs/status/SUPPORT_MATRIX.md`](docs/status/SUPPORT_MATRIX.md) — certified durability scope;
- [`formal/lean/README.md`](formal/lean/README.md) — Lean proof artifacts and refinement gates.

## Toolchains

- Rust: **1.98.1**, edition 2024.
- Lean: **4.34.0**, core-only proof artifacts (no Mathlib dependency).
- Third-party Rust dependencies are vendored under `vendor/` so the workspace can build offline.

## Quick verification

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets --offline
cargo clippy --workspace --all-targets --offline -- -D warnings
cargo test --workspace --all-targets --offline
```

Formal checks:

```bash
./scripts/ci-formal.sh
```

The formal gate requires Lean 4.34.0 locally. GitHub CI installs the pinned Lean toolchain automatically when proof-relevant files change.

## Layout

```text
crates/                 production Rust kernel crates
formal/lean/            Lean proofs + source-refinement binders
vendor/                 vendored Rust dependency closure
artifacts/               retained diagnostics, evidence, certification records
docs/spec/              normative specification
docs/architecture/      current architecture documentation
docs/status/            project status and ledgers
docs/api/               public API design/roadmap
docs/reports/current/   current pass/repository reports
docs/history/           archived pass reports/spec snapshots/manifests
.github/workflows/      Rust and Lean CI
scripts/                local verification/statistics helpers
```

## Development policy

1. The logical/semantic contract is authoritative; physical optimizations must not silently redefine it.
2. Exact semantic equality/order is defined by pinned `Γ`, not by incidental Rust `Eq`/`Hash`/`Ord` where semantic modules apply.
3. Recoverable failure must happen before authoritative publication/commit boundaries.
4. Durability claims are scoped to certified storage profiles; unsupported profiles fail closed.
5. New surface/kernel vocabulary that changes the #20 proof boundary must update the Lean artifact and refinement binder in the same change.
6. New publication fault points or durability protocol changes that affect #18 must update its Lean/refinement boundary.

## License

Workspace crates declare `MIT OR Apache-2.0`. See `LICENSE-MIT` and `LICENSE-APACHE`.
