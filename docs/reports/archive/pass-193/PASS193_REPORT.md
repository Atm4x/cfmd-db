# CFMD Pass193 — hostile nested-algebraic batch-removal closeout

Date: 2026-09-26
Start: 21:23:42 UTC
Useful boundary: 21:43:42 UTC
Hard boundary: 21:47:42 UTC

## Goal

Continue independently from the frozen Pass192 kernel-plan closeout and justify any new refactor only by a concrete hostile finding. The pass preserves the logical/Γ contract, external Rust API, durability protocol, and Legacy/Obsolete scope.

## P193.H1 — repeated nested-algebraic removal rebuild — CLOSED

Hostile inspection found a real asymptotic seam. `AlgebraicNativeColumn::swap_remove_at` can mutate scalar/product carriers directly, but `Sum`, `Option`, `Seq`, `Set`, `Bag`, and `Map` remove through `select_positions`, rebuilding the column. `apply_planned_relation_delta` previously replayed removals one row at a time. A removal batch of size m over n nested-algebraic rows could therefore pay Θ(m*n), including Θ(n²) when m=Θ(n).

The fix keeps representation choice owner-local:

- `AlgebraicNativeColumn::swap_remove_requires_rebuild` reports whether the physical carrier needs projection-based removal.
- private `InstalledRelation::remove_rows` retains ordinary sequential swap-removal for primitive/scalar carriers.
- rebuild-based algebraic batches simulate the existing physical swap-remove survivor order, build one survivor projection, then atomically install staged row-handle, generation, free-slot and logical-link metadata.
- storage delta application no longer knows the algebraic representation or rebuild predicate.
- no new `pub(super)` production seam remains.

Regression `batched_nested_algebraic_removal_rebuilds_once_and_preserves_logical_handles` verifies survivor logical order, stale-handle invalidation, and slot/generation reuse.

## Architecture / inventory

- kernel-plan `pub(super)`: 203 (same as Pass192 baseline).
- production `.rs >= 1000` excluding test-only files: 0.
- active hot-path `PAYER`: 0; the only `PAYER` text remains the legend.
- owner-private P193 helpers have no production consumers outside `InstalledRelation`.
- Legacy/Obsolete scope untouched.
- no external/public Rust API declaration was intentionally added or removed.

## Verification

- `cargo check -p kernel-plan --tests --offline`: PASS.
- `cargo test -p kernel-plan --lib --offline`: 285 passed / 0 failed / 5 ignored.
- strict `cargo clippy -p kernel-plan --all-targets --offline -- -D warnings`: PASS.
- `cargo check --workspace --all-targets --offline`: PASS.
- `cargo fmt --all -- --check`: PASS.
- strict rustdoc (`RUSTDOCFLAGS=-D warnings cargo doc -p kernel-plan --no-deps --offline`): PASS.
- `formal/lean/check_refinement.py`: PASS, 10 fault points.
- `formal/lean/check_surface_refinement.py`: PASS.
- `scripts/verify-repository.sh` (via bash): PASS.

Repository freeze: 2760/2760 manifest entries, 2761 files including the manifest, zero `target/` files, zero Rust toolchain archives, ZIP CRC PASS, frozen workspace ↔ unpacked ZIP: 0 differences.

## Closeout

P193.H1 is CLOSED. The pass removes a concrete quadratic nested-algebraic delta-removal payer without broadening visibility or introducing a parallel maintained path. No normative semantic/specification change was required.
