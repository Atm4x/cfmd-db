# CFMD Pass 13 — native physical runtime slice, compiled semantic predicates, persistent derivative ownership

Date: 2026-09-19
Baseline: Pass12 verified (171 declared tests)
Final: Pass13 verified (179 declared tests)
Wall-clock note: the intended 20-minute cycle was overrun. I did not capture a trustworthy start timestamp and continued after the first runtime benchmark falsified the abstraction-tax expectation, then reran the release gate after a container timeout. No exact 20-minute completion claim is made.

## 1. Problem → PlanIR had no real physical executor

Pass12 proved checked lowering and native-layout preservation, but `PreparedPlan::reference_execute` converted the PlanIR back to `RelExpr` and executed logical semantics. Therefore there was no runtime evidence that physical layout bindings were usable without a mandatory universal-row conversion.

### Hypothesis

Start with a narrow but real executor and keep unsupported operators explicit. In particular, a columnar `Scan -> FilterEqConst -> Project` path should read the predicate column for every row and projected columns only for matches. It must not materialize complete rows before filtering.

### Implementation

`kernel-plan` now contains:

- `NativeRelation::{RowStore, Columnar}`;
- `PhysicalStore`, keyed by `(relation, LayoutId)` and checked against the declared `LayoutFamily`;
- `PreparedPlan::execute_native` with semantic-context pinning;
- physical execution for `Scan`, `FilterEqConst`, `Project`, `Distinct`, baseline `NestedLoop JoinEq`, and `PromoteToBag`;
- a fused physical `Columnar Scan -> FilterEqConst -> Project` path;
- `ExecutionStats { scanned_rows, values_read, output_rows }`;
- explicit `UnsupportedPhysicalPlan` for `Group` and `TopKWithTies` rather than a hidden fallback to logical evaluation.

For the hostile four-row columnar test the fused path reads exactly 6 values: 4 predicate values plus 2 projected values for the two matches. It does not read/materialize unrelated columns.

### Falsification

- Columnar fused output is compared exactly with logical-reference execution.
- RowStore filter/project output is compared exactly with logical-reference execution.
- Native `Distinct` is compared with logical reference.
- Native baseline nested-loop `JoinEq` is compared with logical reference.
- Installing RowStore bytes under a Columnar layout is rejected.
- A prepared `Group` plan is rejected as `UnsupportedPhysicalPlan`, proving there is no silent logical fallback.

### Result

There is now a real physical-runtime boundary. It is intentionally incomplete, but the implemented operators execute from physical storage rather than round-tripping through `RelExpr`.

## 2. Problem → first physical benchmark exposed a large semantic hot-loop tax

The first 100k-row columnar filter/project benchmark was approximately 5.77x slower than a hand-written specialized loop even though physical value reads were already near-minimal. Inspection showed that every row called `SemanticRegistry::equivalent`, repeatedly resolving the pinned Γ module/digest/contract.

### Hypothesis

Versioned semantics should be resolved once at admission/execution boundary and the resulting law should be callable directly in the hot loop. Constant predicates can be bound once as well.

### Implementation

`kernel-semantics` now exposes:

- `ResolvedPrimitiveEquivalence`, obtained only from `(SemanticContext, SemanticRegistry, SemanticId)`;
- `BoundPrimitivePredicate`, created from a resolved primitive law plus a typed constant;
- direct primitive matching for Unit/Bool/I64/F64Bits/TextExact/TextASCII-CI/live entity ID/historical entity ID.

Structural equivalence remains on the generic semantic path; it is not falsely lowered to a primitive matcher.

A direct regression test verifies that the bound ASCII-CI predicate gives exactly the same answers as its resolved semantic contract.

### Falsification / benchmark progression

Same basic workload: 100,000 rows, predicate matches 6,250 rows, one projected payload column.

- first native implementation: ~5.77x hand-written baseline;
- resolve primitive equality once: ~1.75x in the observed single run;
- bind the constant predicate and remove per-row diagnostic-counter mutation: substantially smaller gap;
- final repeatable benchmark uses 9 alternating rounds x 30 iterations each.

Final release benchmark (`PASS13_RUNTIME_BENCH_RAW_FINAL.txt`):

- native min / median / max: 578,479 / 605,310 / 689,065 ns per iteration;
- hand-written baseline min / median / max: 386,057 / 401,565 / 500,382 ns per iteration;
- median ratio: 1.507x;
- native physical reads: 106,250 values = 100,000 predicate reads + 6,250 projected reads.

### Result

The major semantic-registry lookup tax was real and was removed. The remaining ~50.7% median gap is **not closed** and is now a concrete performance target rather than an architectural guess. This benchmark is a microbenchmark of one fused path, not evidence that the whole engine has a 1.507x tax.

Likely remaining contributors include generic `Value` representation/cloning, result-row allocation, predicate enum dispatch and lack of a typed/vectorized column kernel. These are hypotheses for the next pass, not established causes yet.

## 3. Problem → materialized Set support state existed but compatibility derivative API rebuilt it

Pass12 had a correct `MaterializedSetSupportState`, but callers still had to manually own it or use the stateless derivative path which reconstructed supports from the old intermediate result.

### Hypothesis

Make long-lived state a first-class derivative object bound to query and semantic context. It should consume model transitions repeatedly without rebuilding its own support counts.

### Implementation

Added `MaterializedRelDeltaState` for the stateful operators currently justified by support-count semantics:

- `Project(Set)`;
- `Distinct`.

`MaterializedRelDeltaState::build` evaluates/builds state once, pins the semantic context and operator shape, and `apply_model_change` obtains the child delta then updates the existing `MaterializedSetSupportState` in place.

### Falsification

Two sequential hostile tests build state once and then carry it through the transition sequence

`000 -> 011 -> 010 -> 110 -> 000 -> 101 -> 001 -> 111 -> 100`.

At every transition the maintained delta is compared semantically with `rel_delta_by_recompute`:

- Set projection with hidden support multiplicity;
- Distinct with ASCII-CI representative changes/duplicates.

All transitions agree with the oracle.

### Result

Persisted support-count ownership is no longer merely a low-level object. There is now a query-bound, context-bound long-lived derivative API for Set projection and Distinct. Join/TopK/Group still need their own maintained state representations.

## 4. Final gate

Rust 1.98.1 standalone toolchain.

- 179 declared unit/integration tests;
- `cargo test --workspace` — PASS;
- `cargo test --workspace --release` — PASS;
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS;
- `cargo fmt --all -- --check` — PASS;
- `cargo build --workspace --release` — PASS;
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps` — PASS;
- 19 workspace crates;
- 17,183 Rust LOC;
- no Cargo registry/git sources;
- 0 `unsafe`;
- 0 TODO/FIXME;
- 0 `panic!` / `todo!` / `unimplemented!` macros in Rust source.

The chained first release gate hit the container command timeout while compiling; rerunning the warmed release test command completed successfully. This was an infrastructure timeout, not a failing test.

## 5. Remaining problems, ordered

1. **Physical runtime is still partial.** `Group` and `TopKWithTies` are explicitly unsupported; KeyValue/Adjacency/DenseArray/Inverted/Custom are catalog contracts, not implemented runtimes yet.
2. **The measured simple columnar path still has a ~1.507x median gap** to a hand-written specialized loop. The no-mandatory-abstraction-tax target is therefore not yet demonstrated.
3. **Generic non-fused Columnar Scan still row-materializes.** The fused filter/project path avoids this, but a general typed/vectorized column batch representation and wider operator fusion are still needed.
4. **Indexed ΔJoin is not implemented.** Current derivative Join correctness is operator-local replay; physical Join is baseline nested-loop.
5. **TopK maintained state/order statistics are not implemented.** Both delta and physical runtime need a maintained ordering/index structure.
6. **Group maintained aggregate state is not implemented.** In particular ExactF64Sum needs a persistent exact accumulator/provenance strategy for deletions.
7. **Long-lived derivative ownership currently covers only Project(Set)/Distinct.** It must become a compositional state tree for Join/TopK/Group.
8. **OrderedView/pagination** remains a distinct semantic/runtime type to design.
9. **Durability/engine layer** remains: WAL/recovery, persistent indexes/materializations, compaction, concurrency control, crash tests, then distribution.

The frontier has moved from “can PlanIR represent native layouts?” to “can physical kernels/state close the measured runtime gap while preserving the already-checked semantics?”
