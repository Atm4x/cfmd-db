# PASS82 REPORT — HISTORICAL PHYSICAL CONVERGENCE, WAVE A START

Status: **VERIFIED / SOURCE FROZEN**.
Baseline: final Pass81/AZ `cfmd_workspace_pass81_final_write_rnd_converged.zip`.

## Goal

Begin the authoritative 22-row historical backlog one whole problem at a time, rebasing closed R&D contracts onto current Pass81 production rather than copying Pass80 reference workspaces. The Pass81 write calculus remains authoritative and unchanged.

## Result

### Historical #1 — structural/custom semantic persistence + ordering — PROD CLOSED

Pass81 already supplied structural Γ-ordering, semantic order-class keys, checkpoint persistence/reopen and structural TopK tie semantics. Pass82 integrated the missing SAMF ordered/annotation overlay layer:

- `SupportAtomOrderedOverlay<K>` is bound to the exact SAMF revision/catalog/product;
- every live row must be represented exactly once;
- one equality atom must have one order-class key, otherwise construction fails closed;
- `SupportAtomAnnotationOverlay<A>` is atom-scoped and explicitly drops dead-atom state.

This closes #1 without introducing Rust `Ord` or physical tie order as semantic authority.

### Historical #3 — unified physical lifecycle/materialization ontology — PROD CLOSED

- replaced the family-specific internal owner tag with `UnifiedArtifactId` across current optional families;
- retained the existing family-neutral `PhysicalCapability` and common admission kernel;
- added capability projection for unified artifacts;
- added `PhysicalStore::converge_observable_atom_candidate`;
- candidate SAMF state is built/validated before any duplicate retirement;
- advisor-owned semantic index/statistics/quotient-factor duplicates may retire only after replacement publication;
- manual ObservableAtom and manual legacy pins survive convergence.

No new logical/semantic authority was introduced. Specialized Group/TopK/etc. may remain physical lowerings/adapters until their own historical rows are optimized.

### Historical #5 — shared resource/memory pressure — PROD PARTIAL

- shared `ResourceFootprint` atoms now deduplicate only with exactly matching weights;
- conflicting byte weights for one resource atom fail closed via `ResourceFootprintError`;
- external RSS/available-memory observations are represented separately from internal exact resource atoms;
- pressure can reject optional candidate builds but cannot invalidate manual required state.

The remaining gap is not resource semantics: real runtime pressure sampling/telemetry has no production controller consumer yet. That belongs to historical #4 and is deliberately not faked by calling #5 closed.

## Verification

Frozen source delta versus Pass81/AZ is exactly three Rust files:

- `crates/kernel-semantics/src/support_atom.rs`
- `crates/kernel-plan/src/advisor.rs`
- `crates/kernel-plan/src/lib.rs`

Final gate before source freeze:

- `cargo fmt --all -- --check` — PASS
- `cargo check --workspace --all-targets` — PASS
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS
- `cargo test --workspace --all-targets` — PASS
- test discovery: **618 declared tests**; newly added hostile tests are non-ignored
- `unsafe`: 0
- `TODO/FIXME/todo!/unimplemented!`: 0
- workspace-local `target/`: absent

## Exact next checkpoint

**Pass83: historical #4 autonomous advisor/telemetry/controller + completion of #5 runtime pressure plumbing.**

Use the existing Pass82 unified candidate/resource/capability substrate. Add deterministic integer telemetry for read work saved, write-maintenance work and rebuild work; deterministic epoch decay; a non-authoritative maintenance controller; real pressure-sample consumption; and hysteresis/budget hostiles. Do not create the old Pass80 parallel `unified_advisor.rs` ontology. After #4/#5, move to historical #7 recovery economics.
