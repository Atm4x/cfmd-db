# CFMD Pass 20 — compositional typed batch programs + indexed Join downstream batching

Date: 2026-09-19
Baseline: Pass19 verified (200 declared tests)
Final: Pass20 frozen (203 declared tests)
Wall-clock target: 20 minutes
Start: 14:53:57 UTC
End: 15:14:16 UTC
Measured cycle: 20m19s
Implementation frozen before wall-clock boundary; remaining cycle is verification/report/audit/packaging only.

## 1. Problem: Pass19 fusion was still operator-pattern specific

Problem -> Pass19 proved one no-intermediate-row slice, `Project(IndexedI64Join(Scan,Scan))`, but ordinary chains still depended on ad-hoc recognizers. That does not scale into the ideal small PlanIR + typed physical pipeline: adding one fused function per operator tree would recreate a second optimizer/runtime semantics.

Hypothesis -> compile a supported physical subtree into a compact batch program. Project should become column remapping, Filter should become a pre-bound predicate, and materialization into logical `Value` rows should occur only at the final semantic boundary. The same idea should extend downstream of an indexed Join.

Implementation -> `kernel-plan` now has a compositional typed batch path:

- `TypedBatchProgram` handles arbitrary unary chains over `TypedColumnar` containing `Scan`, `FilterEqConst`, `Project`, and `PromoteToBag`;
- projections remap native physical column indices instead of creating rows;
- primitive equality is resolved/bound once before the row loop;
- all filters in the chain execute against native columns;
- final logical `Value` construction happens only for surviving final output columns;
- `typed_batch_chain_hits` makes admission observable in diagnostics.

A separate `IndexedJoinBatchProgram` extends the same physical contract across direct indexed-I64 Join. A parent `Project`, `FilterEqConst`, or `PromoteToBag` is compiled over left/right raw I64 columns and stable join positions rather than over a materialized joined `Vec<Row>`.

Falsification -> the existing nested unary case now exercises `Filter -> Project -> Filter -> Project -> PromoteToBag` composition. Pass20 adds a vertical `IndexedI64Join -> Filter -> Project` case over duplicate keys and payloads. Both are compared exactly against the logical evaluator, and the join-chain case confirms persisted-index reuse, zero ephemeral rebuilds, one batch-chain hit, and correct Bag multiplicity/order.

Result -> the physical executor now has a real compositional batch-program fragment. This is broader than Pass19 pattern fusion, but still not the full DAG: Distinct/Group/TopK, nested joins and mixed-type indexed joins remain outside this batch representation.

## 2. Physical type errors are no longer silently converted to predicate false

During audit, the initial cursor version used a fallible typed predicate inside `retain` and converted any error into `false`. That would make a physical/schema contract violation observationally indistinguishable from a non-matching row.

The batch compiler now validates primitive bound-predicate compatibility before execution and propagates `PhysicalTypeMismatch` instead of hiding it. Empty relations are checked structurally as well, so absence of rows cannot bypass physical type validation.

## 3. I64 batch microkernel: large interpreter tax falsified and mostly removed

The first compositional batch interpreter was correct but slow. On a 100k-row workload with two filters, an intermediate projection and one final projected column:

- generic typed-batch interpreter: ~749 us in the first diagnostic run;
- row-materialized physical runtime: ~2.03 ms;
- hand-written direct I64 loop: ~0.136 ms;
- initial typed/hand-written ratio: ~5.53x.

This falsified any claim that “batch composition alone” removed abstraction tax. The leading cause was repeated `NativeColumn`/predicate dispatch inside the row loop.

Implementation -> the batch program now selects a raw-I64 microkernel when all relevant native columns/predicates are I64. It hoists type dispatch before the scan, passes raw slices, and has direct 0/1/2-predicate loops with a generic fallback for longer chains. `Value::I64` is constructed only for final output rows.

After this specialization, one diagnostic run measured ~316.7 us typed vs ~285.6 us hand-written (1.108x). Three final independent process runs measured:

- process 1: 206,280 ns typed / 129,834 ns hand-written = 1.588x;
- process 2: 193,332 ns / 187,399 ns = 1.031x;
- process 3: 161,604 ns / 127,518 ns = 1.267x;
- median process-level ratio: **1.267x**.

Against the row-materialized executor, the same three process ratios are 0.107x, 0.087x and 0.083x (median 0.087x), i.e. roughly an order of magnitude faster.

Result -> the original 5.53x interpreter tax is not inherent to the compositional batch abstraction. A noisy ~3-59% process-level gap to the hand-written loop remains, so zero mandatory tax is **not** claimed for the general batch program yet.

Evidence: `PASS20_TYPED_BATCH_BENCH_FINAL.txt` plus intermediate Pass20 batch benchmark files retained in the workspace.

## 4. Join read-accounting correction

The old indexed Join stats counted key reads but omitted payload reads needed to materialize result rows. Pass20 corrects `values_read` accounting to include output width. The three-row self-join with five Bag output rows therefore records 6 key reads + 10 payload reads = 16 values rather than the old undercount of 6.

This changes diagnostics only, not logical or physical query results.

## 5. Final verification gate

Rust 1.98.1 standalone toolchain.

- `cargo fmt --all -- --check` — PASS
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS
- `cargo test --workspace` — PASS
- `cargo test --workspace --release` — PASS
- `cargo build --workspace --release` — PASS
- strict rustdoc (`RUSTDOCFLAGS=-D warnings`) — PASS
- release overflow-check tests for `kernel-plan` + `kernel-integration` — PASS
- declared tests: **203/203**
- workspace crates: 19
- Rust LOC: **22,075**
- external Cargo sources: 0
- `unsafe`: 0
- TODO/FIXME: 0
- `panic!` / `todo!` / `unimplemented!`: 0

## 6. Status checklist

### Recently closed before Pass20

- [x] High-churn tombstone growth replaced by reusable generational row slots.
- [x] Stale handles cannot alias reused slots.
- [x] Logical scan order is independent from physical `swap_remove` order.
- [x] Relation/index deltas are prevalidated then committed in place without full-state clone.
- [x] Persisted I64 index accelerates semantic removal lookup.
- [x] First direct `Indexed Join -> Project` no-intermediate-row slice.

### Closed / strengthened in Pass20

- [x] Arbitrary unary `Scan/Filter/Project/PromoteToBag` chains compile as one typed batch program.
- [x] `IndexedI64Join -> Filter -> Project` executes without materializing the full joined logical rowset.
- [x] Physical predicate-type mismatch is propagated, not silently treated as `false`.
- [x] Raw-I64 microkernel selection removes most of the first generic batch-interpreter tax.
- [x] Batch-chain logical ordering survives physical swap-remove/churn.
- [x] Indexed Join diagnostic read accounting includes result payload reads.

### Remaining / newly exposed

- [ ] **Complete the typed batch DAG:** Distinct, Group, TopK, nested/multiway joins and mixed-type indexed joins still cross a logical-row boundary.
- [ ] **Batch constant-factor gap:** final hand-written comparison ranges ~1.03-1.59x across processes (median ~1.27x). Need broader kernels/vectorization without benchmark-specific shortcuts.
- [ ] **Other persisted key types:** Text/collation, Bool, F64 semantics and nominal entity IDs.
- [ ] **General multi-index planning:** choose update/query candidate indexes by semantic compatibility + selectivity/cost.
- [ ] **Peak-capacity slot reclamation:** only if memory benchmarks show historical peak slot capacity matters in practice.
- [ ] **Stateful TopK:** maintained order-statistics + ties.
- [ ] **Stateful Group:** maintained Count and deletion-capable exact/reproducible F64 state.
- [ ] **General long-lived Join/TopK/Group derivative state.**
- [ ] KeyValue / CSR / DenseArray / Inverted / Custom physical runtimes.
- [ ] OrderedView / pagination.
- [ ] WAL/recovery, durable materializations, crash tests and compaction.
- [ ] Transaction-repair runtime, distribution and formal mechanization after the engine layer is credible.

## Bottom line

Pass20 moves typed execution from isolated fused patterns to a compositional batch-program fragment and proves that the large first interpreter tax is an implementation artifact rather than a semantic requirement. The immediate physical frontier is no longer basic Filter/Project composition; it is extending the same raw typed DAG across stateful/set operators and richer joins while closing the remaining noisy constant-factor gap.
