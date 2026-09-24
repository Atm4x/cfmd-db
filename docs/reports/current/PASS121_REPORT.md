# Pass121 — Repository hardening / projectization

## Scope

Pass121 intentionally changes repository organization and project-facing documentation, not kernel semantics.

## Problem → hypothesis → implementation → falsification → result

### Problem

The Pass120 workspace was a valid research snapshot but a poor long-lived repository: hundreds of pass artifacts and versioned spec snapshots lived in the root, the README still described an early research kernel, current status required reconstructing many ledgers, and no repository CI policy existed for the active Lean proof boundary.

### Hypothesis

The kernel can be converted into a conventional Rust project without discarding provenance by separating active source/spec/status from immutable historical artifacts and by adding pinned toolchain/CI/repository policy files.

### Implementation

- production crates, formal artifacts and vendored dependencies retained;
- root reduced to build/toolchain/license/project entry points;
- full current spec promoted under `docs/spec/` with a concise root `SPEC.md`;
- current architecture/status/closed ledger written as project-facing documents;
- old pass reports/diffs/manifests/spec snapshots moved under `docs/history/` by pass;
- raw gate evidence and diagnostics retained under `artifacts/`;
- Pass120 destructive certification evidence retained under `artifacts/certification/`;
- `.gitignore`, Rust/Lean toolchain pins, local CI scripts and GitHub Actions added;
- Lean CI is path-filtered to proof-relevant changes and can also be run manually.

### Falsification / verification

Pass121 must preserve production bytes unless a repository-path refinement binder requires a path-only update. Verification gates are recorded after repository construction in `PASS121_VERIFICATION.md`.

### Result

The workspace is now structured as a project repository suitable for an initial Git commit and RustRover import. Historical provenance remains available but no longer dominates the project root.
