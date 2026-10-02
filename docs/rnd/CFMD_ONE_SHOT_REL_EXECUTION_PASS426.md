# CFMD one-shot relational execution — PASS426

PASS426 extends the P425 `RelExecutionSource -> one-shot operator lowering -> RelExecutionSink` law without introducing a fallback evaluator or maintained-query ownership.

Accepted lowerings now include:

- `FilterEqConst`;
- `FilterOrderConst` using the pinned deterministic ordering module;
- `FilterEqColumns`;
- Bag `Project` as direct projection;
- Set `Project` as projection followed by exact Γ quotient;
- Set/Bag `Union` (Set union quotients in canonical row-key space);
- `Distinct` as exact Γ quotient;
- `Difference` and `AntiJoin` from P425;
- `JoinEq` with one mathematically inherent right-fiber index keyed by `CanonicalEqKey` while the left side streams;
- `PromoteToBag` as representation-free semantic lowering.

`Group` and `TopKWithTies` remain fail-closed until their one-shot aggregate/order state is implemented. There is no routing to the legacy whole-row evaluator.

## Join law

For `JoinEq(L,R,l,r,E)`:

```text
R -> CanonicalEqKey_E(r) -> finite matching fiber
L streams one row at a time
for each left key k:
    emit left × fiber_R[k]
```

The retained state is therefore `O(|R| + output)` for the chosen orientation, not maintained Scan/Watch authority. A future cost-aware orientation may select the cheaper side, but a fixed SQL-style planner is not required for correctness.

## Hostile performance

100k exact-I64 equi-Join release measurements, three iterations per run:

```text
run A generic RelExpr::evaluate: 85.237 / 88.606 / 76.844 ms
run A one-shot:                  116.965 / 67.060 / 60.931 ms

run B generic RelExpr::evaluate: 153.308 / 93.331 / 74.286 ms
run B one-shot:                   70.327 / 68.544 / 62.089 ms
```

The first cold one-shot sample is noisy, but warm one-shot execution is in/below the generic cost class. No P424-style maintained-state payer was observed.

## Remaining Γ evidence seam

Set `Union`, Set `Project`, `Distinct`, and Set `Difference` already compute complete canonical row keys as part of their exact quotient/subtraction semantics. The final `PreparedFactorizedRelation::from_execution_columns -> RelationBaseWitness::build_columnar` currently canonicalizes those output rows again.

PASS426 deliberately does **not** add a public/trusted raw-key constructor merely to skip that work. The next design must carry an opaque certified row-aligned Γ-evidence object whose construction is owned by `kernel-query`; the physical sink may adopt it, but cannot forge it. This preserves one Γ authority while eliminating duplicate output canonicalization.
