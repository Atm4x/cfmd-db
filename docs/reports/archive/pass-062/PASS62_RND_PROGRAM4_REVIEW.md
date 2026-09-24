# PASS62 R&D PROGRAM4 HOSTILE REVIEW

Input: `CFMD_RND_PROGRAM4_CLOSEOUT_PASS60_2026-09-20.zip`.

Decision: **ACCEPT WITH MANUAL REBASE AND CORRECTIONS**.

## Accepted claims

- the current structural algebra has an exact recursive native physical representation;
- typed relations can mix scalar-specialized columns and algebraic structural columns;
- structural selection/mutation can operate on native tags/offsets/payload columns rather than rebuilding the complete column through logical `Vec<Value>`;
- pinned Γ structural canonical keys can be computed directly from algebraic storage;
- the representation is reconstructible physical state, not semantic authority;
- the R&D release diagnostics provide useful fixture-specific evidence that a mandatory `Value` wrapper is not required on the tested hot accesses.

## Rejected as-is / changed during integration

1. **Mechanical Pass60 patch merge.** Rejected because Pass61 already contains Program3 `DenseLiveEntityIds` and later Γ-QCN changes. The patch was rebased semantically.
2. **Pattern-specific structural Filter boundary.** Replaced by a general `TypedBatchPredicateKind::Algebraic` so structural filtering composes with the existing typed-batch DAG and stateful consumers.
3. **Public low-level mutation helpers.** Narrowed to crate scope to preserve `PhysicalStore` as the mutation/stable-handle authority boundary.
4. **Unchecked public recursive TypeExpr admission.** Found hostile counterexample: non-empty `μX.X` can recurse indefinitely. Fixed by validating `TypeExpr` at the public constructor boundary.
5. **R&D overflow gate.** The R&D package explicitly marked it incomplete due its environment timeout. The authoritative Pass62 integration reran the complete overflow-check workspace gate and passed after a warmed retry.

## Additional hostile coverage added

- structural Product<TextAsciiCI> Filter uses the compositional typed-batch path;
- the same structural filter feeds typed Group Count without full input-row materialization;
- algebraic predicate and Program3 DenseLiveEntityIds projection coexist across authoritative mixed remove+insert and physical compaction;
- composed Option/Seq/Set/Bag/Map/Sum native canonical keys exactly match the authoritative registry canonical keys;
- non-empty unguarded recursive type is rejected before descent.

## Remaining boundary

Nested LiveRef leaves inside `AlgebraicNativeColumn` currently lower to external-ID `LiveEntityIds`, not automatically to revision-local `DenseLiveEntityIds`. This is not a semantic defect: both reconstruct the same logical LiveRef identity. It is a future multi-family physical-lowering/advisor choice that should be justified by workload and memory measurements.

Program4 is therefore considered **production integrated/closed at the algebraic physical-family level** in Pass62, while the broad multi-family lifecycle, durable structural encoding and remaining-layout frontiers stay OPEN.
