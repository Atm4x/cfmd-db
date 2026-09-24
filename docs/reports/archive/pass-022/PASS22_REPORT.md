# CFMD Pass 22 — maintained Group delta state

Date: 2026-09-19
Baseline: Pass21 verified (209 declared tests)
Final: Pass22 verified (214 declared tests)
Wall-clock target: 20 minutes
Start: 2026-09-19T16:26:32Z
Frozen/audit boundary: 2026-09-19T16:46:39Z (20m07s)

## 1. Problem

Pass21 allowed Group/Distinct/TopK to consume typed batch selections, but Group still recomputed aggregate state per execution and the derivative path used local replay. There was no long-lived aggregate object that could consume a small child RelationDelta directly.

## 2. Maintained Group state

Added `MaterializedGroupDeltaState`. It is built once from a Group query and initial model, then accepts either model changes or a precomputed child `RelationDelta` through `apply_input_delta`.

This is a compositional boundary: a future owned state tree can connect child delta producers directly to maintained Group without replaying the complete model or complete group input.

Supported aggregates:
- Count with exact count state and checked deletion;
- ExactF64Sum with deletion implemented as an exact inverse update.

Sequential hostile tests compare every maintained transition against `rel_delta_by_recompute` across group births/deaths, duplicate semantic keys, ASCII-case-insensitive grouping, negative/exact floating sums and the global empty-group identity row.

## 3. Atomicity and deletion algebra

`ExactCount` now has checked `remove_one()` and `is_zero()`. Crossing `u64::MAX` promotes to Big representation losslessly; deleting back across the boundary demotes without changing the value. Underflow is an explicit `CountUnderflow` error.

`ExactF64Sum::remove(value)` applies the exact additive inverse and was falsified on ordinary, negative and subnormal finite values.

Input deltas are type/shape checked before mutation. Maintained updates plan only affected buckets, validate the whole transition, then commit. Invalid/malformed or over-removing deltas leave state unchanged.

## 4. Lookup falsifier and compiled I64 Count kernel

The first generic maintained implementation was correct but catastrophically slow for many groups: the 50k-distinct-group benchmark did not finish within 120 seconds because bucket lookup was linear.

The retired design was replaced by:
- persistent `BTreeMap<i64, bucket-index>` for exact-I64 single-key groups;
- `swap_remove` group death plus repair of only the moved bucket mapping;
- no clone of all groups during an update;
- a compiled exact-I64 Count maintenance kernel that dispatches outside the delta hot loop.

A hostile sequential test then exposed a stale-index bug when several groups died in one delta: pre-recorded indices were invalidated by the first `swap_remove`. The commit plan now stores key + next state and re-resolves the current bucket index immediately before each commit operation. The hostile test now passes.

## 5. Performance evidence

Diagnostic: 50,000 simultaneously maintained exact-I64 groups, alternating one-row group death/birth, 7 rounds x 100 iterations.

Five independent frozen-code processes:
- maintained: 739 / 712 / 685 / 767 / 702 ns median-per-process;
- fair hand-written BTreeMap baseline that also constructs logical RelationDelta: 160 / 159 / 153 / 160 / 154 ns;
- process ratios: 4.618x / 4.477x / 4.477x / 4.793x / 4.558x; median process ratio ~4.558x.

Result: asymptotic full-replay and linear-group-lookup taxes are gone; maintained transition latency is sub-microsecond in this diagnostic, but a material constant-factor overhead remains. No no-tax claim is made for maintained Group.

Raw evidence: `PASS22_MAINTAINED_GROUP_BENCH_FINAL.txt`.

## 6. Verification gate

- 214 declared tests;
- `cargo fmt --all -- --check` PASS;
- strict workspace Clippy PASS;
- workspace debug tests PASS;
- workspace release tests PASS;
- release build PASS;
- strict rustdoc PASS;
- overflow-check release tests for kernel-aggregate/kernel-query/kernel-integration PASS;
- 19 workspace packages, 0 external Cargo sources;
- 0 unsafe/TODO/FIXME/panic-shaped macros in production Rust audit;
- 24,442 Rust LOC in crates.

## 7. Status checklist

Recently closed before Pass22:
- [x] typed-batch input for Distinct/Group/TopK;
- [x] generational physical row handles;
- [x] persisted I64 Join index and clone-free physical delta commit.

Closed in Pass22:
- [x] long-lived Group Count state;
- [x] deletion-capable ExactF64Sum maintained state;
- [x] direct child RelationDelta -> Group state composition boundary;
- [x] global empty Group identity transitions;
- [x] atomic malformed/underflowing delta rejection;
- [x] O(groups) exact-I64 lookup retired;
- [x] all-groups clone on update retired;
- [x] stale bucket-index bug under multi-group death fixed;
- [x] compiled exact-I64 Count maintenance kernel.

Remaining/new:
- [ ] maintained TopK/order-statistics with exact ties;
- [ ] one owned compositional Join/Group/TopK state tree;
- [ ] Group/TopK as typed-batch producers, not only consumers;
- [ ] reduce maintained I64 Group ~4.56x fair micro-baseline constant-factor gap;
- [ ] maintained generic/Text grouping needs indexed semantic lookup rather than vector fallback at high cardinality;
- [ ] nested/multiway and mixed-type joins;
- [ ] persisted Text/F64/Bool/entity indexes + cost/selectivity planner;
- [ ] remaining physical layouts and OrderedView/pagination;
- [ ] WAL/recovery/durable materializations/crash tests;
- [ ] transaction repair runtime, distribution, formal mechanization.
