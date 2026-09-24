# IMPLEMENTATION REPORT — PASS85

Pass85 integrates the verified PWRC semantics into the current production architecture and deliberately does not retain the later failed #11 experiment.

## Production changes

`kernel-fixpoint` now owns exact positive recursive Bag proof-tree multiplicity. `BigNatural` preserves arbitrary finite counts and `NaturalInfinity` represents the semantic infinity of grounded productive recursion. The solver first establishes grounded support, classifies productive SCCs iteratively, propagates infinity through live positive dependencies, and evaluates the remaining finite condensation DAG exactly.

`kernel-query` adds a finite-carrier `FixpointCall` boundary with explicit atoms/rules, compact `(Row, NaturalInfinity)` output, shape/carrier validation, and a finite-only rejection path. `kernel-plan` adds `PreparedPositiveRecursivePlan`, pinning the call to the exact semantic context at compilation and rejecting Γ drift at execution.

No row-expansion representation was added. No new durable or logical authority was introduced.

## Pass85 source delta

Exactly five Rust/Cargo files differ from Pass84:

- `crates/kernel-fixpoint/src/lib.rs`
- `crates/kernel-query/src/lib.rs`
- `crates/kernel-query/Cargo.toml`
- `crates/kernel-plan/src/lib.rs`
- `crates/kernel-plan/Cargo.toml`

## Verification

Rust 1.98.1 with external target directory. Final post-rollback workspace fmt/check/strict-Clippy/full-test gate is clean: **634 passed, 0 failed, 8 ignored**. Frozen source fingerprint: `879fccf5d0d474abc8a0d6c612729c32dcf56335ddfb0d90334ae7f7515474ed`.

Historical #2 is therefore **PROD CLOSED**.

## Rejected #11 splice

The old epoch/horizon reference was tested against current production rather than trusted mechanically. It failed because later Γ-REIC gives causal effects a raw-tx-id-derived identity and validates them through the exact retry ledger. This makes new-epoch tx-id reuse collide with causal history and makes retry payload GC incompatible with causal validation. A pre-checkpoint crash also exposes that the old recovery map cannot represent two epoch-qualified uses of one raw id.

The attempted #11 changes to `kernel-durability` were completely reverted before the final verification gate. Pass86 must solve #11 with composite retry identity and a separate replay-stable causal event identity instead of weakening either idempotency or Γ-REIC.
