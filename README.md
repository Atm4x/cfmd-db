# CFMD

CFMD is an experimental embedded/local database kernel built around a constructive finite-model view of state. The repository contains the Rust kernel, revision/change/query machinery, durability/recovery, trust/deployment layers, executable law tests, and Lean artifacts that bind selected architectural claims to the implementation.

## Current status

The historical kernel backlog (#1–#22) is closed for the declared support scope, and the global hostile/refactor campaign is **COMPLETE / FROZEN after Pass280**. Every kernel crate has received either a dedicated hostile closure pass or a grouped audit proportional to its size; the historically heavy `query/plan/semantics/durability` line was revalidated against the final workspace before freeze.

`FROZEN` does not mean “bug-free forever”. It means kernel cleanup is no longer continued by inertia: reopen a frozen area only for a concrete correctness counterexample, proof/authority seam, measured complexity/performance regression, new R&D requirement, or public API/DX requirement.

The active project phase is now **productization**, with a Python-first application facade as the primary product target and a compact Rust facade/runtime service underneath it.

## Product direction

The intended application experience is familiar lazy querying over deep domain paths, with CFMD-specific capabilities around the query:

```python
users = db.users.where(
    lambda u: u.active & (u.passport.country.code == "RU")
)

async for delta in users.watch():
    ...

future = db.preview(plan)
future.delta(users)
future.why_changed(users)
future.commit()
```

The facade rules are stricter than a conventional ORM:

- no hidden storage I/O from ordinary materialized Python attribute access;
- deep relationship traversal is symbolic inside query construction;
- many-valued paths require explicit `any/all/match/aggregate` semantics;
- current, historical and speculative candidate worlds are explicit;
- exact watch is a runtime protocol, not callback-driven polling;
- Plans/Candidates/history use the same authoritative transition pipeline;
- one authoritative runtime owns writes; external Studio/tooling attaches to it rather than editing files independently.

See [`docs/api/PRODUCT_ROADMAP.md`](docs/api/PRODUCT_ROADMAP.md) and the retained design source [`docs/api/CFMD_PYTHON_FACADE_THEORY.md`](docs/api/CFMD_PYTHON_FACADE_THEORY.md).

## Documentation map

- [`SPEC.md`](SPEC.md) — concise repository-facing specification;
- [`docs/spec/CFMD_CORE_SPEC.md`](docs/spec/CFMD_CORE_SPEC.md) — full normative core specification and append-only implementation record;
- [`docs/status/PROJECT_STATUS.md`](docs/status/PROJECT_STATUS.md) — current phase and release boundary;
- [`docs/status/KERNEL_HOSTILE_LEDGER.md`](docs/status/KERNEL_HOSTILE_LEDGER.md) — current kernel audit/freeze inventory;
- [`docs/architecture/ARCHITECTURE.md`](docs/architecture/ARCHITECTURE.md) — current runtime/product layering;
- [`docs/architecture/CRATE_MAP.md`](docs/architecture/CRATE_MAP.md) — internal workspace crate graph;
- [`docs/api/PRODUCT_ROADMAP.md`](docs/api/PRODUCT_ROADMAP.md) — Python-first product roadmap;
- [`docs/api/RUST_API_ROADMAP.md`](docs/api/RUST_API_ROADMAP.md) — supporting Rust runtime/facade roadmap;
- [`docs/reports/current/PASS280_REPORT.md`](docs/reports/current/PASS280_REPORT.md) — kernel-refactor closeout report;
- [`docs/reports/current/RELEASE_PREP_2026-09-28.md`](docs/reports/current/RELEASE_PREP_2026-09-28.md) — repository/CI/documentation release-prep report;
- [`docs/status/SUPPORT_MATRIX.md`](docs/status/SUPPORT_MATRIX.md) — certified durability scope;
- [`formal/lean/README.md`](formal/lean/README.md) — Lean proof artifacts and refinement gates.

## Toolchains

- Rust: **1.98.1**, edition 2024.
- Lean: **4.34.0**, core-only proof artifacts (no Mathlib dependency).
- Third-party Rust dependencies are vendored under `vendor/` for reproducible/offline builds.

## Quick verification

Repository completeness first:

```bash
bash ./scripts/verify-repository.sh
```

Rust gates:

```bash
bash ./scripts/ci-rust.sh
```

Formal/refinement gates:

```bash
bash ./scripts/ci-formal.sh
```

The repository verifier checks required project files, the repository manifest, and literal Rust `include!("...")` targets. A partial upload such as `physical_store.rs` without `physical_store/core.rs` therefore fails before Cargo compilation.

## Layout

```text
crates/                 27 internal Rust workspace crates
formal/lean/            Lean proofs + source-refinement binders
vendor/                 vendored Rust dependency closure
artifacts/              retained diagnostics/evidence/certification records
docs/spec/              normative specification
docs/architecture/      current architecture documentation
docs/status/            project status, support and hostile ledger
docs/api/               product/facade design and roadmaps
docs/reports/current/   current closeout/release-prep reports
docs/reports/archive/   historical pass reports
docs/history/           older provenance/spec snapshots/manifests
.github/workflows/      Rust and Lean CI
scripts/                local verification/statistics helpers
```

## Development policy

1. Logical/semantic contracts are authoritative; physical optimizations must not silently redefine them.
2. Exact semantic equality/order is defined by pinned `Γ`, not incidental Rust `Eq`/`Hash`/`Ord` where semantic modules apply.
3. Recoverable failure must occur before authoritative publication/commit boundaries.
4. Durability claims are scoped to certified storage profiles; unsupported profiles fail closed.
5. Public facades must not expose internal crate ownership/layout merely because those types exist in Rust.
6. Exact watch/Candidate/history/tooling must share the same revision/change semantics rather than grow parallel event systems.
7. New proof-boundary vocabulary or durability fault points must update the corresponding formal/refinement gate.
8. Frozen kernels reopen only on evidence, not cleanup-by-inertia.

## License

Workspace crates declare `MIT OR Apache-2.0`. See `LICENSE-MIT` and `LICENSE-APACHE`.
