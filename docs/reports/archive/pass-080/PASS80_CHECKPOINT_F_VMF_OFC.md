# Pass80 checkpoint F — Γ-VMF publication boundary + Γ-OFC runtime guard

Intermediate safety checkpoint; not final Pass80 VERIFIED release.

## Production changes

1. `RuntimeViolationState` is reconstructible exact Γ-VMF state bound to `(RevisionId, SemanticRevision)`.
2. Runtime bootstrap and candidate revision preparation require `V = 0`.
3. `PreparedRuntimeRevisionTransition::seal` and materialization reconfiguration re-check both VMF revision binding and zero state before acquiring publication authority.
4. VMF witnesses currently cover semantic Set-row uniqueness, missing live-reference targets, and missing capability-required fields. Full validation remains independent authority/oracle during rollout.
5. `RuntimeRevisionBundle::observe_query` captures an exact revision/root-bound Γ-OFC observation guard.
6. OFC prepared-transition impact uses a source-relation routing envelope only as a sound fast path; relevant changes are classified by the pinned Γ-DTC program.
7. Full recomputation remains an independent OFC parity oracle.

## Hostile evidence

- an intentionally corrupted nonzero candidate VMF state is rejected by `seal` before publication; live root is unchanged;
- OFC exact DTC impact agrees with full recomputation for both fiber-preserving and fiber-changing prepared transitions;
- an observation guard cannot be reused against an identical same-RevisionId runtime on another root lineage.

## Verification

- workspace debug tests: 511 passed / 0 failed / 8 ignored;
- `cargo fmt --all -- --check`: PASS;
- `cargo check --workspace --all-targets`: PASS;
- workspace Clippy `-D warnings`: PASS.

Release/overflow/rustdoc remain deferred until final Pass80 source freeze.
