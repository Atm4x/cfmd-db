# CFMD implementation report — through Pass20

Current verified checkpoint: Pass20, 2026-09-19.
Measured implementation/audit cycle: 20m19s (14:53:57–15:14:16 UTC).

## Executive state

The normative logical kernel remains unchanged: `Revision = (Schema S, SemanticEnv Γ, finite Model M)`. Pass20 changes the physical execution compiler/runtime, not database semantics.

Current implementation includes:

- algebraic structural values plus nominal identity and live/historical references;
- explicit Set/Bag/Seq/Map semantics under versioned semantic modules;
- guarded recursive μ equivalence;
- exact closed query IR and exact recomputation derivative fallback;
- fine/operator-local deltas for the current relational IR;
- long-lived Set/Distinct derivative support state;
- lifecycle least-fixed-point normalization;
- identity transport/revision DAG machinery;
- checked certificates + typed PlanIR;
- heterogeneous typed columnar execution;
- generational reusable physical row slots;
- persisted/maintained I64 indexes with atomic relation+index delta application;
- index-assisted update row resolution;
- physical correctness paths for every current `RelExpr`;
- compositional typed batch programs for unary Scan/Filter/Project/Promote chains;
- downstream batch execution above direct indexed-I64 Join for Filter/Project/Promote;
- raw-I64 batch microkernels selected outside the hot row loop.

## Pass20 implementation changes

### Compositional unary batch program

`TypedBatchProgram` compiles supported unary PlanIR chains into native column mappings plus pre-bound predicates. Intermediate Project nodes only remap columns. Multiple Filter nodes accumulate primitive predicates. No intermediate `Vec<Row>` is created; only final surviving projected cells are converted to logical Values.

### Indexed Join downstream batch program

`IndexedJoinBatchProgram` represents direct I64 indexed Join output as left/right physical positions and native column references. Downstream Filter/Project/Promote operators execute on those native columns before final materialization. Persisted right-side indexes are reused; exact ephemeral fallback remains available.

### Specialized I64 microkernel

The first generic batch interpreter was still ~5.53x the hand-written I64 loop. The runtime now resolves the physical scalar kind once and dispatches to a raw-I64 microkernel. Direct loops exist for 0, 1 and 2 predicates, with a general raw-I64 predicate-vector fallback. Final benchmark process ratios to hand-written code were 1.588x, 1.031x and 1.267x; median 1.267x. Versus row-materialized physical execution, typed batch median ratio was 0.087x.

### Diagnostics corrected

Indexed Join `values_read` now includes payload values read during output materialization instead of counting only join keys.

## Verification

Pass20 frozen state: 203 declared tests. Debug/release workspace tests PASS; workspace strict Clippy `-D warnings` PASS; fmt PASS; release build PASS; strict rustdoc PASS; overflow-check release tests for kernel-plan + kernel-integration PASS.

19 crates; 22,075 Rust LOC; zero external Cargo sources; zero `unsafe`; zero TODO/FIXME; zero `panic!`/`todo!`/`unimplemented!` occurrences in Rust sources.

## Immediate implementation frontier

1. extend typed batch representation through Distinct/Group/TopK, nested/multiway and mixed-type joins;
2. lower the remaining ~1.03-1.59x noisy hand-written batch gap with general specialization/vectorization, not workload-specific hacks;
3. add persisted semantic indexes for Text/Bool/F64/entity keys and cost/selectivity planning;
4. maintained TopK and Group state plus broader long-lived derivative ownership;
5. other physical layouts + OrderedView;
6. durability/recovery/crash testing;
7. transaction-repair runtime, distribution and formal closure.

See `CFMD_IDEAL_DB_SPEC.md` for the normative target and `PASS20_REPORT.md` for the detailed problem→hypothesis→implementation→falsification→result chain.
