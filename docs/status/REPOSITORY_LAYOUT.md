# Repository Layout Policy

The project root is reserved for repository-level build/configuration and current entry-point documentation. Historical pass artifacts must not be added back to the root.

- `crates/`: production Rust crates.
- `formal/lean/`: active Lean proofs and refinement binders.
- `vendor/`: committed Cargo vendor closure used by offline builds.
- `artifacts/evidence/`: retained raw historical gate evidence.
- `artifacts/diagnostics/`: retained diagnostic/benchmark helper projects.
- `artifacts/certification/`: support-profile certification evidence.
- `docs/spec/`: normative specification.
- `docs/architecture/`: current and provenance architecture docs.
- `docs/status/`: authoritative current project status and ledgers.
- `docs/api/`: public API design work.
- `docs/reports/current/`: current pass/projectization reports.
- `docs/reports/archive/pass-NNN/`: archived pass reports, diffs, manifests and freeze records.
- `docs/history/spec-snapshots/`: old versioned spec snapshots.
- `docs/history/benchmarks/`: pass-scoped benchmark/matrix directories formerly in the root.

Historical artifacts are immutable provenance. New work should update current docs and add a new report rather than editing old pass evidence.
