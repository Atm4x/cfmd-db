# CFMD Pass 21 — stateful operators consume typed batches + exact-count fast representation

Date: 2026-09-19
Baseline: Pass20 verified/frozen (203 declared tests)
Final: Pass21 verified (209 declared tests)
Finalization wall-clock target: 20 minutes
Finalization start: 16:01:44 UTC
Finalization end: 16:21:58 UTC
Measured finalization cycle: 20m14s

Pass21 source work existed in the interrupted workspace before this finalization cycle. This cycle audited it, removed new production Clippy suppressions, added the ExactCount representation-boundary falsifier, reran every strict gate, refreshed benchmark evidence, updated the ideal specification, and packaged the verified checkpoint.

## 1. Problem: Pass20 batch DAG still stopped before Distinct / Group / TopK

Problem -> Pass20 could keep `Scan/Filter/Project/PromoteToBag` chains and the current indexed-I64 Join downstream path native, but a stateful/set operator forced the complete input to become logical `Vec<Row>` first. That made stateful operators a mandatory materialization boundary even when the input was already typed-columnar.

Hypothesis -> preserve the compiled unary `TypedBatchProgram` and carry only selected physical positions into the stateful operator. Stateful operators may then read exactly the required native columns and materialize logical values only when the operator semantically needs them or when emitting final output.

Implementation -> `TypedBatchSelection` contains:

- the compiled `TypedBatchProgram`;
- selected physical positions in logical scan order;
- execution statistics.

New typed-stateful execution paths:

- `Distinct`: consumes selected positions directly. A one-column exact-I64 specialization uses `BTreeSet<i64>`; the general semantic path materializes candidate rows only when needed for semantic equality.
- `Group Count`: groups selected positions directly. Exact-I64 group keys use native key values and `ExactCount`.
- `Group ExactF64Sum`: reads only the physical group key and aggregate column and keeps the exact/reproducible accumulator.
- `TopKWithTies`: orders physical positions first and materializes only the final retained rows. I64 and F64 ordering have native comparator paths.
- `typed_stateful_batch_hits` proves at runtime/tests that the typed stateful path was actually selected.

Falsification -> four dedicated differential tests compare Distinct, Group Count, Group ExactF64Sum and TopK-with-ties against the logical evaluator while asserting the batch-stateful admission counter. Full workspace tests additionally retain the older row-path correctness oracle. A fifth hostile fallback test uses Text ASCII-case-insensitive equality: `"A"` and `"a"` must collapse in both typed Distinct and typed Group Count according to pinned Γ semantics. This proves the stateful batch layer does not silently substitute host-language/Text exact equality when the I64 specialization is unavailable.

Result -> Distinct/Group/TopK no longer force *input* row materialization. They are not yet arbitrary typed-batch **producers**: their result currently returns to the logical-row boundary. Thus the full compositional batch DAG remains PARTIAL.

## 2. ExactCount no longer pays arbitrary-precision cost for ordinary counts

Problem -> `ExactCount` previously stored `BigUnsigned` from the first increment. Logical exactness requires arbitrary range, but normal Group Count workloads almost never exceed native 64-bit counters.

Implementation -> `ExactCount` now uses:

```text
Small(u64) | Big(BigUnsigned)
```

`add_one` and `merge` stay in `Small` while arithmetic is representable and promote losslessly to `Big` on `u64` overflow. `finish_i64` retains the logical result-range check.

Falsification -> Pass21 adds `exact_count_promotes_losslessly_when_small_representation_overflows`: `Small(u64::MAX).add_one()` and merging `Small(u64::MAX)` with `Small(1)` must both produce the same Big value and report the same final i64 overflow.

Result -> arbitrary precision remains a semantic guarantee but is no longer an ordinary per-group representation tax.

## 3. Production suppression audit

During finalization, the first Pass21 implementation had introduced five additional `#[allow(clippy::too_many_...)]` attributes in production `kernel-plan` code.

Those were removed by splitting the stateful dispatcher and introducing compact `StatefulBatchEnv`, `GroupBatchSpec` and `TopKBatchSpec` parameter objects plus smaller accumulator/finalization helpers.

Final result:

- Pass20 `kernel-plan` Clippy allow count: 13;
- Pass21 final count: 13;
- new suppressions added by Pass21: **0**.

Semantic errors in Distinct fallback remain propagated as errors; they are not converted into `false`/non-match results merely to simplify iterator code.

## 4. Runtime diagnostic evidence

Benchmark: `crates/kernel-plan/examples/stateful_batch_bench.rs`, 100,000 rows, alternating rounds, process-level medians.

Five final process runs:

### Group Count

- 1.028x hand-written BTreeMap baseline
- 1.075x
- 1.104x
- 1.093x
- 1.214x
- median process ratio: **1.093x**

The ~9% median gap is still open implementation/performance debt. The benchmark does not establish a mandatory semantic abstraction tax; it only measures the current physical implementation against this baseline.

### TopKWithTies

- 0.453x current hand-written diagnostic baseline
- 0.480x
- 0.460x
- 0.633x
- 0.540x
- median process ratio: **0.480x**

This is evidence that late materialization/position selection helps this particular workload. It is **not** a claim that CFMD beats an optimal specialist TopK implementation; the current baseline is a diagnostic implementation, not a lower bound.

Raw evidence: `PASS21_STATEFUL_BATCH_BENCH_FINAL.txt` plus diagnostic/repeat files retained in the workspace.

A frozen-code 20-process follow-up (`PASS21_STATEFUL_BATCH_BENCH_20PROC.txt`) gave:

- Group: min 1.030x, median **1.128x**, max 1.288x;
- TopK: min 0.426x, median **0.497x**, max 0.727x.

The wider sample reinforces the same interpretation: Group has a real but modest current constant-factor gap; TopK late materialization is advantageous for this diagnostic workload, with substantial process noise.

## 5. Final verification gate

Rust 1.98.1 standalone toolchain.

- `cargo fmt --all -- --check` — PASS
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS
- `cargo test --workspace` — PASS
- `cargo test --workspace --release` — PASS
- `cargo build --workspace --release` — PASS
- strict rustdoc (`RUSTDOCFLAGS=-D warnings`) — PASS
- release overflow-check tests for `kernel-aggregate` + `kernel-plan` + `kernel-integration` — PASS
- declared tests: **209/209**
- workspace crates: 19
- Rust LOC: **22,117**
- external Cargo sources: 0
- `unsafe`: 0
- TODO/FIXME: 0
- `panic!` / `todo!` / `unimplemented!`: 0

The combined all-in-one gate command hit the environment timeout while recompiling the overflow profile; the overflow gate was immediately rerun separately on the same frozen source and passed.

## 6. Status checklist

### Recently closed before Pass21

- [x] Generational reusable slots bound high-churn row-handle metadata.
- [x] Relation/index deltas are prevalidated and committed without full-state clone.
- [x] Persisted I64 index accelerates query execution and removal candidate lookup.
- [x] Compositional unary typed batch programs for Scan/Filter/Project/PromoteToBag.
- [x] Indexed-I64 Join can feed downstream Filter/Project without full joined-row materialization.

### Closed / strengthened in Pass21

- [x] Distinct consumes typed batch selections directly.
- [x] Group Count consumes typed batch selections directly.
- [x] Group ExactF64Sum reads only required typed columns.
- [x] TopKWithTies sorts/selects physical positions before materialization.
- [x] Ordinary ExactCount uses native `u64` and losslessly promotes only when needed.
- [x] Representation-boundary overflow has a hostile test.
- [x] Text ASCII-CI Distinct/Group fallback preserves pinned Γ equality, not host equality.
- [x] Pass21 adds no new production Clippy suppressions.

### Remaining / newly exposed

- [ ] **Stateful operators as typed-batch producers.** Distinct/Group/TopK consume batches but currently emit logical rows; downstream native composition still breaks there.
- [ ] **Maintained Group state.** Current Group recomputes per execution; need delta-maintained Count and deletion-capable exact/reproducible F64 state/provenance.
- [ ] **Maintained TopK/order statistics.** Current path selects/sorts per execution; need long-lived threshold/tie state.
- [ ] **General long-lived Join/Group/TopK derivative ownership.** Persisted I64 Join index is only one maintained artifact fragment.
- [ ] **Group constant-factor gap.** 20-process frozen audit median is ~1.13x hand-written baseline (range 1.03-1.29x).
- [ ] Nested/multiway joins and mixed-type indexed joins inside the typed batch DAG.
- [ ] Persisted indexes for Text/collation, Bool, F64 semantics and nominal entity IDs.
- [ ] General query/update index cost/selectivity planner.
- [ ] KeyValue / CSR / DenseArray / Inverted / Custom physical runtimes.
- [ ] OrderedView / pagination.
- [ ] WAL/recovery, durable materializations, compaction and crash tests.
- [ ] Transaction-repair runtime, distribution and formal mechanization after durability is credible.

## Bottom line

Pass21 removes the first stateful/set **input** materialization wall from the typed physical pipeline and eliminates the ordinary BigUnsigned tax from exact Group Count. The next frontier is no longer "can Group/TopK read typed data?"; it is whether those operators can become composable typed/maintained artifacts that stay native across downstream operators and across revisions.
