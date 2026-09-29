# Repository Layout Policy

The project root is reserved for build/configuration and current entry-point documentation. Historical pass artifacts must not be added back to the root.

- `crates/`: 34 Rust workspace crates: 28 internal kernel/infrastructure crates, public SDK `cfmd` + `cfmd-derive`, and the runtime/protocol/host/transport product stack;
- `formal/lean/`: active Lean proofs and refinement binders;
- `vendor/`: committed Cargo vendor closure used by offline builds;
- `artifacts/evidence/`: retained raw historical gate evidence;
- `artifacts/diagnostics/`: retained diagnostic/benchmark helper projects;
- `artifacts/certification/`: support-profile certification evidence;
- `docs/spec/`: normative core specification/implementation journal;
- `docs/architecture/`: current and provenance architecture docs;
- `docs/status/`: authoritative current project status, support and hostile ledger;
- `docs/api/`: product facade theory and implementation roadmaps;
- `docs/reports/current/`: current kernel-closeout/release-prep reports;
- `docs/reports/archive/pass-NNN/`: archived pass reports, diffs, manifests and freeze records;
- `docs/history/spec-snapshots/`: old versioned spec snapshots;
- `docs/history/benchmarks/`: pass-scoped benchmark/matrix directories formerly in the root.

Historical artifacts are immutable provenance. New work updates current docs and adds a current report rather than rewriting archived pass evidence.

## Completeness policy

`bash ./scripts/verify-repository.sh` is the first repository gate. It verifies required entry-point files, literal Rust `include!("...")` targets, Cargo vendor checksum metadata, and the repository manifest. This is intended to fail clearly on partial GitHub uploads before Cargo compilation.
