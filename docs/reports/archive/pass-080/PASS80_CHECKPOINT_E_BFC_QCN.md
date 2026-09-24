# Pass80 checkpoint E — Γ-BFC structural QCN repair

Intermediate safety checkpoint; not final Pass80 VERIFIED release.

## Production changes

1. GCC supports structural reconciliation across append-only atom universes and changed hyperrule sets/bodies while remapping still-valid selected witnesses.
2. Γ-BFC lowers changed support programs to changed death programs and repairs only the invalidated witness cone.
3. Materialized Γ-QCN persists BFC atom identity by `(leaf, StableRowHandle)` including generation; slot reuse cannot alias stale atoms.
4. QCN insertion/resurrection and deletion now share the BFC/GCC structural repair path. The old bespoke deletion path is test-only oracle code.
5. Retained-memory accounting includes BFC program/certificate/atom-map state.

## Hostile evidence

- 2,000 randomized structural program mutations: incremental == full rebuild.
- physical slot reuse with incremented generation obtains a new BFC atom; old atom remains a separate tombstone.
- QCN insertion resurrection touches a strict subset of BFC atoms rather than the entire materialized support universe.
- all Γ-QCN targeted tests agree with reference/fresh rebuild.

## Verification

- `kernel-grounded-closure`: 11 passed / 0 failed.
- targeted `kernel-plan gamma_quotient_`: 13 passed / 0 failed.
- full workspace fmt/check/tests/clippy `-D warnings`: PASS.

Release/overflow/rustdoc remain deferred until final Pass80 source freeze.
