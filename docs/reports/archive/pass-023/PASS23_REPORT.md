# CFMD Pass 23 — maintained TopKWithTies ordered state

Date: 2026-09-19
Baseline: Pass22 verified source tree (source audit: 216 declared tests; Pass22 report stated 214 and undercounted by 2)
Final: Pass23 frozen source (219 declared tests)
Wall-clock target: 20 minutes
Start: 2026-09-19T16:51:11Z
Frozen/audit wall-clock boundary: 2026-09-19T17:11:19Z (20m08s). Source was frozen before the boundary; no implementation changes occurred during final stress/audit.

## 1. Problem

Pass22 gave Group a long-lived delta-maintained state, while TopKWithTies still depended on replay/sort semantics whenever the input changed. The execution-time typed TopK from Pass21 avoided early row materialization but did not own an order-statistics state across revisions.

The target for this pass was stronger than a cached result: a TopK state must consume a small child `RelationDelta`, preserve exact tie semantics under the declared ordering, reject inconsistent deltas atomically, and avoid work proportional to the complete input for the exact-I64 fragment.

## 2. Maintained TopK state

Added `MaterializedTopKDeltaState`.

It is built once from a `TopKWithTies` query plus an initial model and can then accept either:

- `apply_model_change(old, change, ...)`, deriving the child delta through the existing exact incremental machinery; or
- `apply_input_delta(&RelationDelta, ...)`, allowing direct parent/child state composition without replaying the model.

The state is bound to the exact pinned `SemanticContext`, input `RelType`, ordering ID, direction and `k`.

### Exact-I64 path

For the current I64 ordering domain the state stores:

```text
BTreeMap<i64, tie-bucket<Row>>
```

One input insert/delete therefore touches only the affected key bucket. Reading TopK walks keys in ascending or descending order only until at least `k` rows are covered, and it includes the complete threshold bucket. Exact ties therefore remain exact rather than being truncated to exactly `k` rows.

Tie-bucket row order is stable under insert/delete so the implementation does not acquire accidental physical-map ordering inside equal keys.

### Generic semantic fallback

Text/F64/non-I64 orderings use a semantic sorted-row fallback. Insert position is determined with the pinned ordering module, and row removal uses the input relation's declared semantic equivalences. This path is correctness-grade but not yet asymptotically performance-grade: vector insertion/removal remains O(n).

A Text ASCII-case-insensitive hostile test verifies that semantic ties are preserved by Γ rather than Rust `String` equality/order.

## 3. Atomicity and hostile falsifiers

New sequential falsifiers cover:

- ascending exact-I64 threshold jumps;
- descending exact-I64 threshold jumps;
- tie bucket birth/death and complete threshold-bucket retention;
- transitions through empty input;
- generic Text ASCII-CI ties;
- missing removal rejected before state mutation.

Every sequential transition is compared against `rel_delta_by_recompute` through semantic RelationDelta equivalence.

Direct input deltas first check:

1. pinned semantic context equality;
2. exact input `RelType` equality;
3. row value/type shape;
4. removal existence / Set duplicate constraints before commit.

For pinned maintained state, repeated semantic-module admission checking was removed from the hot delta path: module/law admission already happened at build time and context equality prevents semantic drift. Row shape is still validated on every externally supplied delta.

## 4. Performance falsification

Diagnostic workload: 50,000 exact-I64 input rows, `k=10`, alternating one-row deletion/insertion that moves the threshold.

The first maintained implementation was correct but measured about 2.96 us/transition against a ~0.105 us tiny hand-written BTreeMap baseline. Generic semantic output diff was identified as one avoidable tax.

A one-column exact-I64 output-delta path now compares raw key multiplicities rather than invoking semantic row equality for every TopK output candidate. Pinned module re-validation was also removed from the hot path.

Five final frozen-code process runs:

- maintained: 2.550 / 2.487 / 2.426 / 2.500 / 2.371 us;
- tiny ordered-map baseline: 0.111 / 0.111 / 0.119 / 0.120 / 0.120 us;
- maintained/baseline: 22.97x / 22.41x / 20.39x / 20.83x / 19.76x.

The constant-factor gap is therefore still large and remains explicit optimization debt.

However, a full-replay/remove+sort baseline over the same 50k input measured about 0.314-0.360 ms per transition. Maintained execution is about 0.7% of that replay time, roughly 100-150x faster. This demonstrates that the full-input replay tax is removed without claiming the maintained microkernel is close to the specialist lower bound.

Raw evidence:

- `PASS23_MAINTAINED_TOP_K_BENCH_INITIAL.txt`;
- `PASS23_MAINTAINED_TOP_K_BENCH_FAST1.txt` ... `FAST4.txt`;
- `PASS23_MAINTAINED_TOP_K_BENCH_FINAL.txt`;
- `PASS23_MAINTAINED_TOP_K_BENCH_REPEATS.txt`.

## 5. Verification gate

Frozen source passes:

- workspace debug tests PASS;
- strict workspace Clippy PASS;
- `cargo fmt --all -- --check` PASS;
- workspace release tests PASS;
- workspace release build PASS;
- strict rustdoc PASS;
- overflow-check release tests for kernel-query/kernel-integration PASS;
- 19 workspace packages, 0 external Cargo sources;
- 0 unsafe/TODO/FIXME/panic-shaped macros;
- 25,217 Rust LOC in `crates`;
- kernel-query production Clippy allow count unchanged at 1.

Test-count audit correction: Pass22's report said 214 declared tests, but a direct source grep of the preserved Pass22 workspace yields 216. Pass23 adds exactly three new `#[test]` functions, producing 219. The implementation state is unaffected; this is a reporting-count correction.

## 6. Result

TopK is no longer an execution-only/replay operator for the exact-I64 fragment. It now owns persistent ordered state and accepts child RelationDelta directly, with exact ties, ascending/descending ordering and atomic error behavior verified against recomputation.

The remaining gap has shifted from missing maintained semantics to two narrower issues:

1. generic Text/F64 order-statistics still use O(n) vector fallback;
2. exact-I64 tiny-delta packaging remains ~20-23x a hand-written microbaseline despite being ~100-150x faster than full replay.

No logical database primitive changed in this pass.

## 7. Status checklist

Recently closed before Pass23:
- [x] maintained Group Count;
- [x] deletion-capable ExactF64Sum;
- [x] direct child RelationDelta -> Group state;
- [x] persisted I64 Join state and generational physical handles.

Closed in Pass23:
- [x] long-lived `MaterializedTopKDeltaState`;
- [x] direct child RelationDelta -> TopK state;
- [x] exact-I64 persistent ordered tie buckets;
- [x] exact ascending/descending threshold changes;
- [x] complete threshold-tie retention;
- [x] generic Text ASCII-CI semantic ordering fallback;
- [x] atomic missing-removal rejection;
- [x] pinned-state hot path avoids redundant Γ module re-admission;
- [x] raw one-column I64 output delta path.

Remaining/new:
- [ ] generic Text/F64 maintained TopK needs indexed/order-statistics state rather than O(n) vector insertion/removal;
- [ ] reduce exact-I64 maintained TopK ~20-23x tiny microbaseline constant-factor gap;
- [ ] one owned compositional Join/Group/TopK state tree;
- [ ] Group/TopK as typed-batch producers for downstream operators;
- [ ] reduce maintained I64 Group ~4.56x fair microbaseline gap;
- [ ] generic/Text Group needs indexed semantic lookup at high cardinality;
- [ ] persisted Text/F64/Bool/entity indexes and cost/selectivity planner;
- [ ] nested/multiway/mixed-key joins;
- [ ] remaining physical layouts + OrderedView/pagination;
- [ ] WAL/recovery/durable materializations/crash tests;
- [ ] transaction repair runtime, distribution and formal mechanization.

## 8. Frozen-state audit

After source freeze, a fresh copy created without the original `target/` rebuilt and passed release `kernel-query + kernel-integration` tests and strict `kernel-query --all-targets` Clippy. The frozen release test binary then completed 1,500 exact-I64 threshold/tie sequence replays plus 500 generic Text semantic-tie replays without failure.

No implementation source changed during this audit.

Additional frozen audit: workspace all-targets release tests PASS; `kernel-query` release tests PASS under both `RUST_TEST_THREADS=1` and `16`; package audit reports 19 workspace packages and 0 external Cargo sources. The frozen TopK release tests then passed 3,000 additional targeted replays (2,500 exact-I64 threshold/tie sequences and 500 atomic missing-removal sequences), for 5,000 targeted maintained-TopK replays total after source freeze.

Final frozen stress total: 24,000 targeted maintained-TopK release replays after source freeze (exact-I64 threshold/ties, generic Text semantic ties, and atomic missing-removal cases combined), all PASS.
