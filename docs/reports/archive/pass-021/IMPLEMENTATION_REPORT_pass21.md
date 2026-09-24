# CFMD implementation report — through Pass21

Current verified checkpoint: Pass21, 2026-09-19.
Pass21 finalization/audit cycle: 16:01:44–16:21:58 UTC (20m14s).
Declared tests: 209. Workspace crates: 19. Rust LOC: 22,117.

## Executive state

The normative logical kernel remains unchanged:

```text
Revision = (Schema S, SemanticEnv Γ, finite Model M)
```

Pass21 changes only physical execution and aggregate representation. It does not add a new logical database primitive.

Current implementation now includes:

- algebraic structural values, nominal identities and live/historical references;
- explicit Set/Bag/Seq/Map semantics under versioned semantic modules;
- guarded recursive μ equality;
- closed total query IR and universal exact recomputation/change fallback;
- current relational fine-delta paths plus long-lived Set/Distinct support state;
- lifecycle least-fixed-point normalization;
- identity transport and immutable revision DAG machinery;
- proof/certificate boundaries and minimal PlanIR;
- heterogeneous typed columnar storage;
- generational reusable physical row handles with logical scan order separated from dense physical order;
- persisted and RelationDelta-maintained I64 Join index;
- prevalidated clone-free physical relation/index update commit;
- index-assisted semantic removal lookup;
- compositional typed batch programs for unary Scan/Filter/Project/Promote chains;
- downstream indexed-I64 Join batch execution;
- **typed-batch input execution for Distinct, Group and TopK**;
- ExactCount native-u64 fast representation with lossless BigUnsigned promotion.

## Pass21 implementation delta

Only these source areas differ materially from Pass20:

- `crates/kernel-plan/src/lib.rs`;
- `crates/kernel-plan/examples/stateful_batch_bench.rs`;
- `crates/kernel-aggregate/src/lib.rs`;
- Pass21 benchmark/report/spec evidence.

### Typed stateful boundary

`TypedBatchSelection` reuses the Pass20 batch compiler and carries selected physical positions into stateful operators. `typed_stateful_batch_hits` makes this admission testable.

Distinct:
- exact single-column I64 fast path uses raw native values;
- semantic fallback preserves declared equivalence modules.
- hostile Text ASCII-CI coverage confirms `"A"`/`"a"` collapse through Γ semantics on the non-I64 fallback.

Group:
- exact I64 grouping can operate on raw keys;
- Count uses ExactCount;
- ExactF64Sum reads only group-key and sum columns and preserves exact reproducibility.

TopK:
- operates on physical positions;
- I64/F64 primitive ordering avoids general semantic dispatch in the hot comparator where valid;
- result rows are materialized only after threshold/tie selection.

Important limitation: stateful operators currently return logical rows, not another typed batch object. Therefore the typed DAG is still partial at stateful **output** boundaries.

### ExactCount representation

Exact logical count semantics are unchanged, but storage is now:

```text
Small(u64) | Big(BigUnsigned)
```

Promotion occurs only when native arithmetic overflows. Boundary tests prove increment and merge promotion agree.

### Quality audit

The initial Pass21 state added production Clippy suppressions to accommodate large helper functions. Final code refactors those helpers so Pass21 adds zero suppressions relative to Pass20 (13 `kernel-plan` allows in both checkpoints).

## Benchmark state

`stateful_batch_bench` final five-process evidence:

- Group Count: 1.028x, 1.075x, 1.104x, 1.093x, 1.214x baseline; median 1.093x.
- TopK diagnostic: 0.453x, 0.480x, 0.460x, 0.633x, 0.540x baseline; median 0.480x.
- 20-process frozen audit: Group median **1.128x** (1.030-1.288x), TopK median **0.497x** (0.426-0.727x).

Interpretation:

- Group still has a measurable constant-factor implementation gap.
- TopK benefits strongly from late materialization on this workload, but the benchmark baseline is diagnostic rather than an optimal specialist lower bound.

## Verification

Final frozen source passes:

- fmt check;
- strict workspace Clippy;
- debug workspace tests;
- release workspace tests;
- release workspace build;
- strict rustdoc;
- overflow-checked release tests for kernel-aggregate/kernel-plan/kernel-integration.

Audit: 0 external Cargo sources, 0 unsafe, 0 TODO/FIXME, 0 panic!/todo!/unimplemented!.

## Current highest-priority frontier

1. Make Distinct/Group/TopK typed-batch **producers**, not terminal logical-row boundaries.
2. Add long-lived maintained Group Count and deletion-capable ExactF64Sum state.
3. Add maintained TopK/order-statistics with exact ties/order semantics.
4. Generalize maintained Join state beyond the persisted right-side I64 index fragment.
5. Extend typed/native joins to nested/multiway and non-I64 semantic keys.
6. Add cost/selectivity planning across multiple compatible persisted indexes.
7. Add remaining physical layout families and OrderedView/pagination.
8. Then durability: WAL/recovery, durable materializations, compaction/crash tests.
9. After durability: observation/transaction repair runtime, distribution and formal proof closure.
