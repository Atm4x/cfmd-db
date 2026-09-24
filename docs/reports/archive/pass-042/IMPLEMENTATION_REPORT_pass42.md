# IMPLEMENTATION REPORT — Pass42

## problem

Persisted semantic indexes were single-column only; direct mixed/multi-key equality joins could not use one exact persisted composite lookup; and installed indexes were not governed by an explicit scan-vs-index work decision.

## hypotheses

1. The existing canonical equality law composes naturally as an ordered vector of primitive canonical keys without inventing a new semantic authority.
2. A general column-to-column equality filter is the correct logical representation for secondary equality predicates rather than a special multi-key Join opcode.
3. A direct two-way Join plus cross-side equality predicates can be normalized against a composite right-side persisted index while rechecking Γ equality on returned rows.
4. A simple explicit work model is preferable to unconditional index preference or a magic row-count threshold; index creation/retention should remain a separate planner problem.

## implementation

- generalized `SemanticIndexBinding` to `key_parts: Vec<SemanticIndexKeyPart>`;
- materialized physical semantic indexes now use `Vec<CanonicalEqKey>` keys and one resolved primitive equivalence per component;
- added `SemanticBucketIndex::distinct_key_count()`;
- added logical/maintained/physical `FilterEqColumns` with typechecking, evaluation, delta propagation, transport rewrite and durable metadata encoding;
- added direct filter-chain matching against composite persisted indexes;
- added direct two-way multi-key Join normalization/fusion against composite persisted indexes, including binding-key order normalization;
- added `SemanticAccessCostModel` for scan vs already-installed persisted index on direct Filter/Join;
- added `persisted_index_cost_rejections` execution evidence.

## hostile falsification

- unprofitable tiny Text index is rejected and scan remains exact;
- selective Text index is used;
- mixed TextAsciiCI + I64 composite Filter index matches logical reference and survives delta maintenance;
- mixed-key Join uses one composite index and matches logical evaluation;
- physical result rows are revalidated against every semantic equality predicate;
- structural/custom equivalence remains outside primitive canonical-index closure;
- no new semantic authority is introduced by the cost model or index state.

## verification/result

Final Rust 1.98.1 fmt/check/debug/release/strict-Clippy/release-build/strict-rustdoc/overflow-release gate: PASS.

Metrics: 319 declared tests, 91 `kernel-plan`, 69 `kernel-query`, 32 `kernel-durability`, 21 crates, 42,772 Rust LOC, 0 external Cargo sources, 0 `unsafe`.

## rejected routes

- no tuple-specific semantic primitive; composite physical keys are vectors of already-certified primitive canonical keys;
- no dedicated `JoinEq2/JoinEqN` logical opcode; secondary key conditions use general `FilterEqColumns`;
- no unconditional persisted-index preference;
- no magic dataset-size threshold taken from one benchmark;
- no claim that direct two-way multi-key fusion solves general multiway join planning;
- no automatic index creation/retention policy hidden inside execution.

## recommended next step

Move one level up to an index/join advisor: durable/reconstructible statistics, index create/retain/share/evict/rebuild policy, and multiway join-order enumeration. Keep residual I64 Group/TopK constant-factor work as an independent measured optimization track.

## remaining risks

See `PASS42_REPORT.md` §8. The immediate planner risks are multiway join ordering and index lifecycle economics; structural/custom canonicalization and broader physical layouts remain open. Durability/distribution assurance items remain tracked unchanged.
