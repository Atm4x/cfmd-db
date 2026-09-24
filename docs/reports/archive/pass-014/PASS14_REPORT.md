# CFMD Pass 14 — typed native columns, near-baseline fused kernel, physical Group/TopK

Date: 2026-09-19
Baseline: Pass13 verified (179 declared tests)
Final: Pass14 verified (184 declared tests)
Wall-clock target: 20 minutes

## 1. Typed native columns close the measured Value-storage tax

Problem -> Pass13's final 100k-row Columnar `Scan -> FilterEqConst -> Project` runtime was still about 1.507x the hand-written specialized baseline. The physical read count was already optimal (100,000 predicates + 6,250 projected values), so the remaining leading hypothesis was generic `Value` column representation rather than mandatory semantic lookup.

Hypothesis -> physical layouts may use typed native columns while the logical/query boundary continues to expose exact `Value` semantics. Start with `I64Columnar`; specialize the admitted `I64Exact` predicate once outside the row loop and materialize `Value::I64` only for output rows.

Implementation -> `NativeRelation::I64Columnar` stores `Vec<Vec<i64>>`. The fused I64 kernel compares raw `i64` values and supports a single-column projection fast path while retaining the general projection path. Generic `Columnar<Vec<Value>>` remains available. A typed layout is checked against the pinned schema at execution; an I64 physical column under a non-I64 relation is rejected as `PhysicalTypeMismatch`.

Falsification -> typed and generic columnar layouts are executed for the same query and compared exactly against the logical reference; outputs and physical read accounting agree. A hostile Text relation backed by `I64Columnar` is rejected.

Performance -> the first typed run measured 284,503 ns vs 270,930 ns (1.050x). Earlier repeated typed runs before the final single-projection specialization were roughly 1.073x-1.157x. On the final gated code, three independent processes each ran 9 alternating rounds x 30 iterations:

- process 1 median: 260,351 ns CFMD vs 260,366 ns baseline = 0.999x;
- process 2 median: 297,627 ns vs 256,678 ns = 1.159x;
- process 3 median: 256,389 ns vs 251,293 ns = 1.020x;
- median process-level ratio: 1.020x;
- physical reads remain exactly 106,250 values for 100,000 rows and 6,250 matches.

A post-package replay from a freshly extracted ZIP ran five additional independent processes and produced ratios 1.107x, 1.009x, 1.007x, 1.120x and 0.942x (median 1.009x). This corroborates near-baseline behavior while also showing material scheduler/cache noise between processes.

Result -> the large Pass13 gap was not a necessary semantic-abstraction tax; typed physical representation removes nearly all of it for this microkernel. This is still one I64 fused workload and the inter-process spread is nontrivial, so it is not a universal no-tax theorem.

## 2. Pinned execution API removes redundant caller-context validation from the hot path

`PreparedPlan` now exposes `execute_native_pinned(store, registry)`, which executes against the immutable `SemanticContext` already owned by the prepared plan. The prior `execute_native(store, context, registry)` remains as a compatibility boundary and still rejects a mismatched context before delegating to the pinned path.

Benchmarking showed that removing the full-context equality check did not materially explain the Pass13 gap by itself. It is retained because it is the cleaner ownership model, not because of a performance claim.

## 3. Physical Group and TopK are now implemented

Problem -> Pass13 explicitly rejected `Group` and `TopKWithTies` at physical runtime.

Implementation -> `GroupAlgorithm::LinearReplay` now executes physical child rows directly and supports:

- `Count`, including the empty global-group identity;
- `ExactF64Sum` through `kernel-aggregate::ExactF64Sum`.

`TopKAlgorithm::FullSort` now executes physical child rows directly with fallible semantic ordering and preserves threshold ties. It is a correctness-first insertion-sort implementation, not an order-statistics implementation.

Falsification -> physical Group Count (including empty global group), physical ExactF64Sum, and TopK-with-ties are compared exactly with logical reference execution.

Result -> every current `RelExpr` operator has a native physical correctness path. Performance-grade Group/TopK state remains open.

## 4. Runtime structure cleanup

The physical executor was split into explicit kernels rather than suppressed with `clippy::too_many_lines`: project/fusion dispatch, typed-I64 fused kernel, generic Value-columnar kernel, generic I64 fallback, row-store fused kernel, Group and TopK kernels. Strict Clippy passes without new broad lint suppression.

## 5. Final gate

Rust 1.98.1 standalone toolchain.

- 184 declared unit/integration tests;
- `cargo test --workspace` — PASS;
- `cargo test --workspace --release` — PASS;
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS;
- `cargo fmt --all -- --check` — PASS;
- `cargo build --workspace --release` — PASS;
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps` — PASS;
- 19 workspace crates;
- 17,869 Rust LOC;
- no Cargo registry/git sources;
- 0 `unsafe`;
- 0 TODO/FIXME.

Raw final benchmark evidence is in `PASS14_RUNTIME_BENCH_FINAL.txt`; earlier diagnostic runs are also retained.

## 6. Remaining problems, ordered

1. **Typed physical kernels exist only for I64.** Bool/F64/Text/entity IDs and mixed typed column batches still use generic `Value` storage or row materialization.
2. **The no-mandatory-abstraction-tax target is only demonstrated for one microkernel.** Final I64 filter/project is near baseline, but broader scans, multi-column projections, joins, grouping and ordering need specialized baselines and stable benchmark suites.
3. **Generic non-fused Columnar Scan still materializes rows.** A general typed/vectorized batch interface and wider operator fusion are needed.
4. **Physical Join is still nested-loop.** An indexed/hash/merge join family plus planner choice is needed.
5. **TopK is correctness-first full/insertion sort.** Maintained order statistics and an efficient physical TopK kernel remain open.
6. **Group is correctness-first replay.** Maintained aggregate state, especially deletion-capable ExactF64Sum provenance/accumulator state, remains open.
7. **Long-lived derivative ownership covers Project(Set)/Distinct only.** Join/TopK/Group need compositional maintained state.
8. **Other catalog layouts remain contracts only:** KeyValue, AdjacencyList, DenseArray, Inverted, Custom.
9. **OrderedView/pagination** remains a distinct semantic/runtime type.
10. **Durability/engine layer** remains: WAL/recovery, persistent indexes/materializations, compaction, concurrency control, crash tests, then distribution.

The immediate frontier is now broader typed/vectorized physical kernels plus indexed/stateful operators, not missing logical semantics.
