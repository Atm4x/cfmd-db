# CFMD Pass288 — mixed semantic revision with incremental physical publication

Date: 2026-09-28
Start: 11:49:07 UTC
Useful boundary: 12:09:07 UTC
Hard boundary: 12:13:07 UTC

## Goal

Remove the P287 entity-write hot-path rebuild without weakening lifecycle/reference correctness. Entity/object Plans change relation rows together with lifecycle/carrier/field state; the runtime needs one atomic mixed semantic transition that preserves the exact target Revision while updating physical relation state incrementally.

## P288.MIXED-REVISION — first-class kernel-plan transition — CLOSED

Added `kernel_plan::MixedRevisionTransitionRequest`.

Contract:

- semantic context remains pinned;
- target is an already validated `kernel_revision::Revision`;
- supplied `RevisionRelationMutation`s must exactly explain every changed relation endpoint;
- untouched relations must remain identical;
- lifecycle, carrier and field state may differ from the source;
- duplicate/unbound relation mutations fail closed.

`RuntimeRevisionBundle::prepare_mixed_revision` applies relation deltas to a clone of the existing physical store, updates semantic-quotient supports and affected materializations incrementally, advances touched relation base witnesses, and installs the exact target Revision into the candidate runtime root.

Because non-relation model state changed, P288 does not transport the relation-only violation certificate. It rebuilds `RuntimeViolationState` from the exact target Revision and requires zero before sealing publication.

## P288.PUBLICATION — no full physical rebuild on entity Plan — CLOSED

`cfmd-runtime::Database::commit` now uses `commit_mixed_revision()` for Plans carrying object/entity contracts instead of `replace_revision()`.

The previous P287 endpoint was logically correct but `prepare_full_revision()` rebuilt a fresh `PhysicalStore` from every relation in the target Revision. P288 keeps the same logical endpoint and lifecycle authority while applying only declared relation deltas to existing physical layouts/materializations.

Kernel regression verifies that a transition changing both lifecycle/entity-field state and relation content returns:

`RuntimePublicationEffect::Incremental(..)`

rather than `Rebuilt`.

## P288.DURABILITY — exact recovery retained — VERIFIED

P288 deliberately does **not** introduce a new WAL format.

The mixed runtime descriptor publishes incrementally in memory, but `durable_descriptor()` still records the exact full target Revision bytes. Therefore:

- durable retry identity remains exact;
- same transaction + same mixed target returns `AlreadyCommitted`;
- recovery/reopen reconstructs the identical target Revision;
- no new backward/forward WAL codec compatibility surface was added.

A compact versioned lifecycle/carrier/field durable delta remains an optimization payer. It is not required for runtime correctness and should only replace the exact target encoding after an extensional-equivalence/recovery proof.

## Hostile checks

- mixed target relation endpoints are replay-checked against the supplied deltas;
- untouched relation changes are rejected;
- semantic-context changes still require the full rebuild/schema-migration path;
- non-relation invariant closure is rebuilt from the exact target rather than incorrectly reusing relation-only evidence;
- physical support/materialization maintenance remains affected-only;
- no threshold routing or error-driven fallback added.

## Verification

- `cargo test -p kernel-plan durable_mixed_revision_updates_lifecycle_and_relations_incrementally --offline`: PASS;
- mixed transition reopen regression: PASS;
- mixed transition idempotent retry: PASS (`AlreadyCommitted`);
- `cargo test -p cfmd-runtime --all-targets --offline`: **9 passed / 0 failed**;
- `cargo check --workspace --all-targets --offline`: PASS;
- strict workspace Clippy (`-D warnings`): PASS;
- `formal/lean/check_refinement.py`: PASS, **10 fault points**;
- `formal/lean/check_surface_refinement.py`: PASS.

## Files materially changed

- `crates/kernel-plan/src/runtime_impl/types.rs`;
- `crates/kernel-plan/src/runtime_impl/bundle/revision_prepare.rs`;
- `crates/kernel-plan/src/runtime_impl/bundle/cell_prepare.rs`;
- `crates/kernel-plan/src/runtime_impl/bundle/cell_durable.rs`;
- `crates/kernel-plan/src/runtime_impl/durable/runtime_commit.rs`;
- `crates/kernel-plan/src/test_parts/segment_07.rs`;
- `crates/kernel-plan/src/lib.rs`;
- `crates/cfmd-runtime/src/runtime.rs`;
- `README.md`;
- `SPEC.md`;
- `docs/spec/CFMD_CORE_SPEC.md`;
- `CHANGELOG.md`.

## P289 recommendation

Return to product DX now that entity writes no longer rebuild the physical store. Highest-value next layer: identity lookup (`get/require`) and first-class `Plan -> Candidate -> preview/query/delta -> commit` semantics. Keep compact mixed WAL encoding as a durability optimization payer unless measurement shows the exact target payload is a practical bottleneck.
