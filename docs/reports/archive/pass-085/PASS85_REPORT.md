# PASS85 REPORT — POSITIVE RECURSIVE BAG / PWRC

Status: **VERIFIED / SOURCE FROZEN**.
Baseline: frozen Pass84 `cfmd_workspace_pass84_durable_semantic_core_recovery.zip` (`a3bbbc90548158d1639742a91f565127be83c7d5901dce7886d80cabad5c747e`).

Wall-clock integration start: **2026-09-22 20:03:59 UTC**.
Source freeze: **2026-09-22 20:24:12 UTC** (20m13s). No production Rust source was edited after freeze.

## Goal and result

Pass85 rebases historical #2 / PWRC onto the current Pass84 production spine without replacing late query/plan architecture. Historical **#2 is now PROD CLOSED**.

The production contract is: certified finite carrier -> grounded least support -> exact proof-tree multiplicity in `N∞`; finite multiplicities are arbitrary precision, grounded productive SCCs are compact `∞`, ungrounded cycles remain zero, and no executor expands multiplicity into repeated rows.

## Integrated surface

- `kernel-fixpoint`: `BigNatural`, `NaturalInfinity`, positive Bag program/certificate, iterative SCC/productivity analysis and exact finite proof-tree counting.
- `kernel-query`: `PositiveRecursiveRowAtom`, `PositiveRecursiveRowRule`, `FixpointCall`, compact recursive Bag result, carrier validation and explicit `NonFiniteRecursiveMultiplicity`.
- `kernel-plan`: Γ-pinned `PreparedPositiveRecursivePlan`; execution rejects semantic-context drift.
- repeated premises remain multiplicative; dead conjunctive premises do not falsely activate cycles.
- finite counts never saturate into semantic infinity.

## Hostile verification

Covered finite independent oracle parity, 2,000 randomized acyclic programs, duplicate-premise multiplicity, productive vs ungrounded cycles, dead conjunctive premise, arbitrary-precision finite counts, 20k-carrier iterative traversal, atom-outside-carrier rejection and Γ drift.

Final frozen gate after reverting the unsuccessful #11 experiment:

- `cargo fmt --all -- --check` — PASS
- `cargo check --workspace --all-targets` — PASS
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS
- `cargo test --workspace --all-targets` — PASS
- **634 passed / 0 failed / 8 ignored**
- no new `unsafe`, TODO/FIXME, `todo!` or `unimplemented!` in the Pass85 source delta

Frozen source delta versus Pass84 is exactly five files:

- `crates/kernel-fixpoint/src/lib.rs`
- `crates/kernel-query/src/lib.rs`
- `crates/kernel-query/Cargo.toml`
- `crates/kernel-plan/src/lib.rs`
- `crates/kernel-plan/Cargo.toml`

Frozen Rust/Cargo source fingerprint: `879fccf5d0d474abc8a0d6c612729c32dcf56335ddfb0d90334ae7f7515474ed`.

## Hostile finding on attempted #11

The portable #11 implementation was attempted only after #2 had fully passed. It was **not** retained. On the current architecture, reusing a numeric transaction id under a new idempotency epoch collided with the post-Pass81 Γ-REIC causal ledger because `RevisionEffectId` is derived from raw transaction id; the second effect became self-dependent. The old reference also keys recovered transaction history only by raw id, so a crash before the post-GC checkpoint cannot represent old and new epoch instances simultaneously. Finally, causal validation currently depends on retained exact transaction intent, which conflicts with payload GC.

All #11 source changes were therefore reverted before the final gate. There is no partial/broken #11 implementation in Pass85.

## Exact next checkpoint

**Pass86: historical #11 transaction retry retention/GC, redesigned for the current Γ-REIC architecture.**

Required design boundary:

1. retry identity is `(IdempotencyEpoch, ClientTransactionId)`, including WAL scan/recovery and metadata;
2. `RetryHistoryExpired` is a terminal answer below the durable watermark and can never mean “new transaction”;
3. causal `RevisionEffectId` is independent of raw retry id and is replay-deterministic across crash recovery;
4. durable causal records remain self-contained after exact retry payload GC (including causal-parent/cut information needed for validation);
5. numeric transaction-id reuse in a new epoch survives a crash **before** the next checkpoint;
6. bounded metadata and live retry exactness remain hostile gates.
