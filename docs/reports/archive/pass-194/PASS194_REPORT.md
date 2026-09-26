# CFMD Pass194 — global kernel-plan hostile closeout

Date: 2026-09-26
Closeout audit timestamp: 22:11 UTC

## Goal

Perform a crate-wide hostile audit of `kernel-plan` after the Pass189–Pass193 ownership/split/performance closeout. The audit is not allowed to refactor merely to reduce file or visibility counts: every production change must close a concrete correctness, atomicity, or asymptotic seam. Legacy/Obsolete scope remains untouched.

## P194.H1 — stale derived-artifact dependency topology after QCN materialization — CLOSED

The global mutation audit found one serious correctness defect.

`PhysicalStore::derived_artifacts_by_relation_layout` is a lazily materialized `OnceLock` cache. Normal artifact mutation helpers invalidate it. `materialize_semantic_quotient_artifacts`, however, installed newly built `semantic_quotient_factors` and `semantic_quotient_supports` directly into their owner maps without invalidating an already-warmed dependency cache.

A legal sequence therefore existed:

1. warm the relation/layout dependency contour;
2. materialize a new QCN quotient factor/support;
3. apply a later relation delta;
4. consume the stale dependency contour and fail to schedule the new quotient derivative for maintenance.

This could leave a reconstructible QCN artifact stale even though the authoritative relation had advanced.

Regression `quotient_materialization_invalidates_warmed_derived_dependency_cache` was written first and reproduced the defect: after materialization, the warmed contour contained neither the new `SemanticQuotientFactor` nor `SemanticQuotientSupport` dependency.

The commit boundary now invalidates the dependency topology before publishing changed quotient artifacts. The staging clone used while constructing support also invalidates its inherited cache before injecting newly prepared factors, so support construction observes its staged topology rather than the source snapshot's old cache.

The regression passes after the fix.

## P194.H2 — mutation followed by transition-epoch failure — CLOSED

The same hostile audit reviewed all `TransitionEpochExhausted` sites for error atomicity. Two advisor paths and the quotient materialization path could previously mutate reconstructible state and only then perform the checked epoch increment:

- single observable-atom convergence;
- unified observable advisor publication;
- semantic quotient artifact materialization.

Those paths now preflight the next transition epoch before any authoritative mutation whenever the operation is known to change state. A returned epoch-exhaustion error can no longer represent a partially published mutation in these paths.

The unified observable retirement helper was correspondingly simplified: mutation need is determined before commit rather than reconstructed as an after-the-fact boolean while mutating.

## P194.H3 — accidental quadratic control-plane membership/search — CLOSED

The asymptotic pass found two low-severity but unnecessary quadratic patterns:

- advisor commit loops classified selected artifacts with `Vec::contains` over the growing `created/rebuilt` report vectors;
- multi-relation QCN support maintenance linearly searched the entire revision-change batch for every quotient leaf.

Advisor classification now uses an ordered set of prepared bindings, giving O(log k) membership instead of repeated O(k) report scans. QCN batch maintenance builds one `(relation, layout) -> delta` ordered map and performs logarithmic leaf lookup instead of O(leaves * changed-relations) linear matching.

These were control/topology-plane costs rather than row-count hot paths, but there is no reason to retain accidental quadratic behavior.

## Global hostile audit result

The rest of the crate-wide scan did not produce another substantive OPEN seam.

Verified areas include:

- ownership / visibility boundaries;
- direct mutation bypasses of reconstructible artifact maps and cache invalidation;
- transition-epoch error ordering;
- active `fallback`, `generic`, and `legacy` paths;
- row/join/filter/group/distinct/TopK fallback asymptotics;
- maintained semantic fibers and QCN support maintenance;
- `OnceLock` cache invalidation points;
- production `unwrap`/`expect` invariants;
- suspicious `position`/`find`/`contains` calls inside loops;
- storage-to-multiway and QCN-owner dependency boundaries.

The remaining generic row fallbacks are deliberate Γ-aware correctness paths after typed/persisted routes decline; they are not SQL nested-loop substitutions. The remaining `LegacyIndex` semantic fiber is an explicit compatibility representation and the old QCN support-mask implementation is `#[cfg(test)]` only.

No active row-count quadratic payer was found after Pass193. Topology/query-arity linear searches that remain are bounded structural/planning operations rather than hidden relation-cardinality scans.

## Architecture / inventory

- external Rust declarations: **589 / 589** versus Pass193;
- normalized path-aware declaration SHA used by this audit: `d2eaf301c37ada51d950a583814c110e7de1f080a5fc20705a021c43fb020a31` on both Pass193 and Pass194;
- `kernel-plan` `pub(super)`: **203**, unchanged;
- `HOSTILE[...]`: **74**;
- production `.rs >= 1000` excluding test-only files: **0**;
- active hot-path `PAYER`: **0**; only the marker legend contains the token;
- direct production `storage_impl -> multiway` module edges: **0**;
- `semantic_quotient_physical -> PhysicalStore`: **0**;
- `semantic_quotient_physical -> Plan`: **0**;
- `semantic_quotient_physical -> MultiwayJoinPredicate`: **0**;
- Legacy/Obsolete scope: untouched.

Modified kernel-plan files relative to frozen Pass193:

- `src/multiway/tests.rs` — warmed-cache regression;
- `src/storage_impl/physical_store/capabilities.rs` — QCN dependency-cache invalidation, epoch preflight, batch lookup cleanup;
- `src/storage_impl/physical_store/semantic_advice.rs` — epoch preflight and prepared-binding set membership;
- `src/storage_impl/physical_store/install_advice.rs` — prepared-binding set membership;
- `src/storage_impl/advisor_runtime/unified.rs` — epoch preflight / commit simplification.

No external/public API declaration changed.

## Verification

- `cargo check -p kernel-plan --tests --offline`: PASS;
- `cargo test -p kernel-plan --lib --offline`: **286 passed / 0 failed / 5 ignored**;
- strict `cargo clippy -p kernel-plan --all-targets --offline -- -D warnings`: PASS;
- `cargo check --workspace --all-targets --offline`: PASS;
- `cargo fmt --all -- --check`: PASS;
- strict rustdoc (`RUSTDOCFLAGS=-D warnings cargo doc -p kernel-plan --no-deps --offline`): PASS;
- `formal/lean/check_refinement.py`: PASS, **10 fault points**;
- `formal/lean/check_surface_refinement.py`: PASS.

## kernel-plan closeout status

`kernel-plan` is CLOSED for the present cleanup/hostile-refactor objective.

This is stronger than the Pass192 structural closeout: Pass194 re-audited the whole crate after the Pass193 performance fix and found one real cache-coherency correctness hole plus smaller atomicity/asymptotic defects, all now closed. Continuing to reduce visibility, split files further, or replace deliberate generic correctness fallbacks without a new falsifying case would be refactoring for its own sake.

A future `kernel-plan` pass is justified by a new feature/R&D requirement, a measured performance regression, or a concrete hostile counterexample—not by the current kernel-plan cleanup plan.

## Next hostile targets

Read-only workspace inventory makes `kernel-query` the next target.

`kernel-query` directly owns the exact logical query IR and reconstructible maintained-plan semantics consumed by `kernel-plan`. Its `src/lib.rs` is **18,147 lines** and contains a very large production block before the main test module, including maintained Join/Group/TopK/blocker state and flat maintained-plan graph machinery. It should receive the same hostile treatment before changing physical execution again: ownership split, generic-vs-native semantic paths, delta asymptotics, duplicate maintained representations, and test-driven visibility.

Second target: `kernel-durability`. Its `store.rs` is **7,983 lines** and its durable/WAL/publication state machine has a much larger correctness blast radius than ordinary algorithmic kernels. It should be audited after query semantics are clean, with emphasis on authority boundaries, atomic error paths, recovery/compatibility code, WAL scan complexity, and old format paths.

Third tier after those two: `kernel-semantics` (semantic authority, `lib.rs` 5,958 lines), then `kernel-change` (change/rewrite calculus, one 4,896-line file). `kernel-integration` is primarily the cross-layer falsification boundary and should be reviewed alongside those owners rather than mechanically split first.
