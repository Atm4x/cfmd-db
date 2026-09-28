# Changelog

## Unreleased

### Global kernel closeout (Pass280)

- completed/froze the global kernel hostile/refactor campaign for the current declared scope;
- revalidated the historically heavy query/plan/semantics/durability line against the final workspace;
- completed final model/schema ownership cleanup and bidirectional subtype-closure authority;
- retained an evidence-driven reopen policy rather than continuing cleanup by inertia.

### Repository / CI release preparation

- reconciled GitHub-side workflow fixes so shell gates run through explicit `bash` and do not depend on executable-bit preservation;
- restored Cargo-vendor `.gitignore` exceptions for `Cargo.toml.orig` and `.cargo-checksum.json`;
- strengthened repository verification to catch missing literal Rust `include!("...")` targets and incomplete/corrupt Cargo vendor snapshots before compilation;
- made `ci-rust.sh` run repository-integrity verification before Rust gates;
- refreshed current README/spec/status/architecture/report documentation;
- added a current kernel hostile/freeze ledger.

### Product roadmap

- changed product sequencing from “standalone Rust application facade first, Python later” to a Python-first application facade backed by a stable Rust runtime/facade boundary;
- added the Python facade design source and product roadmap covering deep symbolic queries, exact watch, Plan/Candidate/history, local tooling/Studio and later language surfaces.

### Repository projectization (Pass121)

- reorganized pass-oriented research snapshot into a conventional project repository;
- preserved historical reports/evidence under `docs/history/` and `artifacts/`;
- recorded 22/22 historical problems closed for the declared support scope;
- added Rust/Lean CI policy and toolchain pins.
