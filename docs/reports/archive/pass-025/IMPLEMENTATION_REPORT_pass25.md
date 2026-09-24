# CFMD implementation report — through Pass21

Current verified checkpoint: Pass23, 2026-09-19.
Pass23 finalization/audit cycle: 16:51:11–17:11:19 UTC (20m08s).
Declared tests: 219. Workspace crates: 19. Rust LOC: 25,217.

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

# Pass 22 update — maintained Group state

Problem -> Group had typed-batch execution but no long-lived delta-maintained aggregate state.

Hypothesis -> persist aggregate buckets and consume child RelationDelta directly; deletion must be a first-class exact transition, not local replay.

Implementation -> added MaterializedGroupDeltaState, ExactCount checked decrement, ExactF64Sum inverse removal, affected-bucket atomic planning, adaptive exact-I64 BTree lookup and compiled I64 Count maintenance kernel.

Falsification -> sequential Count/ExactF64Sum/global-group transitions match full recompute; malformed/underflowing deltas are atomic; a 50k-group benchmark falsified vector lookup (>120 s), and a later hostile multi-group-death test found stale indices after swap_remove. Both were fixed.

Result -> Group Count and ExactF64Sum now have verified long-lived maintained state. Frozen diagnostic transition is ~0.7 us at 50k groups but remains ~4.56x the fair hand-written micro-baseline, so constant-factor optimization remains open.

# Pass 23 update — maintained TopKWithTies state

Problem -> TopK had a late-materialized typed execution path but no long-lived state; every incremental change still fell back to replay/sort semantics.

Hypothesis -> keep ordering state across revisions and consume child RelationDelta directly. For exact I64, ordered tie buckets should make update work depend on the changed keys and TopK output size rather than total input size.

Implementation -> added `MaterializedTopKDeltaState`. Exact-I64 ordering uses persistent `BTreeMap<i64, tie-bucket<Row>>`; generic ordering uses a semantic sorted-vector fallback. Both paths preserve exact threshold ties, direction and input Set/Bag semantics. Direct delta application validates pinned context/type/row shape and is atomic on inconsistency. One-column exact-I64 output delta generation bypasses generic semantic row-diff calls.

Falsification -> sequential ascending/descending threshold changes, tie births/deaths, empty input, Text ASCII-CI semantic ties and missing-removal atomicity all compare against the recompute oracle. A benchmark showed that semantic correctness alone was not enough: initial maintained execution remained ~28x a tiny hand-written baseline. Hot-path refinements reduced it to roughly 20-23x but did not close the constant-factor gap.

Result -> full replay tax is removed. On 50k rows/k=10, maintained updates are ~2.4-2.6 us versus ~0.31-0.36 ms full replay, about 100-150x faster, while remaining far above the ~0.11-0.12 us specialist microbaseline. Generic non-I64 order-statistics and microkernel packaging remain open.

Verification -> 219 declared tests after correcting a historical Pass22 report undercount (preserved Pass22 source contains 216, not 214); debug/release workspace tests, strict Clippy, fmt, release build, strict rustdoc and overflow-check query/integration all pass. 0 unsafe/TODO/FIXME/panic-shaped macros; 25,217 Rust LOC.

Frozen-state audit -> fresh-copy release query/integration tests + strict query Clippy passed without reuse of the original target directory. The frozen release test binary then passed 2,000 targeted maintained-TopK replays (1,500 exact-I64 threshold/tie sequences and 500 generic Text semantic-tie sequences).

Additional frozen audit -> workspace all-targets release PASS, query tests PASS at 1 and 16 test threads, 19 local workspace packages / 0 external Cargo sources, and 3,000 further targeted maintained-TopK release replays (5,000 total after freeze).

Final Pass23 frozen stress total: 24,000 targeted maintained-TopK release replays after source freeze, all PASS.

# Pass 24 update — maintained Join and owned state tree

Problem -> maintained Group and TopK existed independently, while Join had no first-class two-input maintained state and no owner propagated child deltas across Join→Group→TopK.

Hypothesis -> persist Join input buckets, emit exact Join RelationDelta from two leaf deltas, then feed that delta directly into Group and TopK. Restrict the first owned tree to the exact-I64 total fast fragment so the hot path does not need whole-tree staging clones.

Implementation -> added MaterializedJoinDeltaState with exact-I64 BTree key buckets plus generic Γ-semantic fallback; added MaterializedJoinGroupTopKState for exact-I64 Join→Group Count→I64 TopKWithTies. Two-sided simultaneous changes are planned per affected key and preserve bag multiplicity. Production helper complexity was refactored without new production Clippy suppressions.

Falsification -> two-sided I64 Join, Text ASCII-CI fallback and sequential owned-tree leaf changes all match full recompute. Initial benchmark mistakenly exposed O(n) TopK because WITH TIES legitimately returned all equal-count groups. With bounded output, maintained updates remain microsecond-scale across 1k→10k state. A separate build-path falsifier found initial state construction still re-evaluates nested children and can hit O(n²) logical Join materialization.

Result -> the previous open owned Join/Group/TopK state-tree item is closed for the exact-I64 fast fragment. New highest-priority stateful issue is compositional child-snapshot construction; generic state ownership/generalization remains open.

Verification -> 222 declared tests; debug/release workspace tests, strict Clippy, fmt, release build, strict rustdoc and overflow query/integration all PASS. 19 local workspace packages, 0 external sources, 0 unsafe/TODO/FIXME/panic-shaped macros, 26,147 Rust LOC. Frozen targeted stress: 900 Join + 1,100 owned-tree release replays PASS.

## Pass25 — compositional state construction + local generic Join maintenance

Problem → parent maintained states rebuilt nested logical children; generic Join used full clone/full replay.

Hypothesis → build parents from child snapshots; maintain generic Join by changed-row algebra `ΔL×oldR + nextL×ΔR`.

Implementation → child-snapshot constructors for Group/TopK; owned tree consumes Join/Group snapshots; generic Join now prevalidates both sides, derives local output delta, then commits atomically.

Falsification → 50k initial tree build completes (~74.9 ms); snapshot-built states equal standalone builds; ASCII-CI simultaneous insert/removal Join deltas equal recompute oracle; full strict gate green.

Result → two Pass24 OPEN items closed. Generic Join still scans the opposite side without a semantic index, so its remaining cost is O(|Δ|·n), not full replay O(n²).

Pass25 verification metadata: wall clock 17:49:38 → 18:09:54 UTC (20m16s); 222 declared tests; frozen targeted generic-Join/state-tree replays: 2,000; 19 local packages; external sources 0; allow(clippy) count unchanged at 17.
