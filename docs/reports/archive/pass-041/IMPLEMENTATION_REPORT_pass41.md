# IMPLEMENTATION REPORT — Pass41

## problem

Stateful typed execution terminated at Group/TopK output, so downstream operators re-entered row representation. Exact-I64 maintained Group and TopK also retained substantial constant-factor overhead, and hostile work exposed an incorrect descending-I64 physical TopK threshold.

## hypotheses

1. Group/TopK can produce an internal owned typed batch without introducing a new logical type or authority layer.
2. Pinned primitive canonical equality keys are sufficient for non-I64 typed Group output while structural/custom semantics must keep exact fallback.
3. Exact scalar I64 TopK does not need full Row buckets; key multiplicities are sufficient for exact top-k-with-ties maintenance.
4. Removing redundant lookup/allocation work should reduce, but must not be assumed to eliminate, the I64 Group/TopK micro-gaps.

## implementation

- added internal `OwnedTypedBatch { columns, positions, stats }`;
- Group and TopK now compose as typed producers through downstream Group/TopK/Filter/Project for the current builtin primitive fragment;
- raw TopK compacts selected positions without cloning the complete input columns;
- primitive Group uses pinned `CanonicalEqKey` values and emits typed Count/ExactF64Sum columns;
- introduced exact scalar-I64 count-only maintained TopK storage and specialized tiny-delta output diff;
- optimized the exact-I64 Group singleton-key replacement path with `ExactCount::is_one()` and slot reuse;
- corrected descending I64 TopK order-statistic selection.

## hostile falsification

- TextAsciiCaseInsensitive Group producer checked against logical semantics;
- F64Total TopK producer checked against logical semantics;
- both `Group -> TopK` and `TopK -> Group` compositions remain typed;
- descending top-k hostile ties now produce exactly the correct threshold set;
- structural/custom equivalence remains fallback rather than being assigned an invented canonical key;
- release/debug assertion mutation audit did not find a recurrence of the Pass39 release-only bug class.

## benchmark/result

`evidence/pass41/STATEFUL_OPERATOR_BENCH.log`:

- maintained I64 Group: ~1.94–2.00x hand baseline, improved from Pass40 ~2.3–2.4x and historical ~4.8x;
- maintained exact scalar I64 TopK: ~6.3–7.3x hand baseline, improved from historical ~20–23x;
- both historical constant-factor gaps remain OPEN.

## rejected routes

- no fixed planner threshold inferred from a single microbenchmark;
- no canonical-key invention for structural/custom semantics;
- no new logical relation/value type for typed producer batches;
- no claim that a smaller but still material constant-factor gap is CLOSED.

## recommended integration / next step

Use the now-compositional typed stateful DAG as the execution substrate for composite/mixed-key physical semantic indexes and multi-key Join lowering. Introduce an explicit cost/selectivity contract rather than unconditional index preference or magic row-count constants.

## remaining risks

See `PASS41_REPORT.md` §9. The immediate data-plane risks are residual I64 constant factors, composite/mixed-key index/planning, structural/custom canonicalization, index lifecycle/cost policy, and COW runtime candidates. Broader durability/distribution assurance remains tracked but was not modified in Pass41.
