# IMPLEMENTATION REPORT — Pass43

## problem

Pass42 could choose scan versus an already-installed compatible semantic index, but current primitive persisted semantic indexes still had no production lifecycle boundary for create/rebuild/retain/share/evict decisions. A naïve automatic policy would also risk claiming ownership of externally installed indexes or publishing a sequence of partial physical mutations.

## hypotheses

1. Index lifecycle is reconstructible physical policy and must not enter `Revision=(S,Γ,M)` authority.
2. A prospective build should be charged once against aggregated expected work for one exact `SemanticIndexBinding`.
3. Advisor ownership must be distinct from index existence: an external compatible index may be reused but not evicted by the advisor.
4. Reconciliation should prebuild/validate every selected missing/stale index and then publish one physical transition/root.
5. A deterministic key-cell budget is useful groundwork but must not be misreported as exact byte memory accounting.

## implementation

- added `SemanticIndexWorkloadSample`, `SemanticIndexAdvisorPolicy` and `SemanticIndexAdvisorReport`;
- added `PhysicalStore::advise_semantic_indexes(...)` for the current builtin primitive semantic-index family;
- discovers direct Filter/composite-Filter and direct equality/composite-Join opportunities below ordinary unary plan nodes;
- aggregates gross expected savings per exact Γ-bound `SemanticIndexBinding` and charges prospective build work once;
- chooses profitable candidates deterministically under `max_managed_key_cells`;
- added explicit `advisor_managed_semantic_indexes` ownership so externally installed compatible indexes are reusable but never advisor-evictable;
- selected stale/missing indexes are completely prepared before any reconcile mutation;
- physical transition epoch advances once per changed reconciliation and not on a no-op;
- `RuntimeRevisionCell` publishes a changed advice result through one immutable runtime-root replacement; `DurableRuntime` exposes the same physical operation;
- existing compatible index statistics are borrowed rather than cloned for advisor analysis.

## hostile falsification

- two individually unprofitable samples jointly amortize one shared build;
- one-shot build remains rejected when expected saving does not repay construction;
- zero managed budget evicts advisor-owned state but preserves/reuses an external index;
- same `SemanticId` with changed module contract forces Γ-bound rebuild;
- mixed TextAsciiCI + I64 composite Join build matches logical reference execution;
- an eligible access path below `Project` is discovered;
- one physical reconcile publishes one new runtime root; repeated no-op advice does not republish;
- debug and release advisor tests pass;
- no side-effecting assertion pattern is present in the advisor/reconcile path.

## verification/result

Final Rust 1.98.1 fmt/check/debug/release/strict-Clippy/release-build/strict-rustdoc/overflow-release gate: PASS.

Metrics: 326 declared tests, 98 `kernel-plan` tests, 21 crates, 43,838 Rust LOC, 0 external Cargo sources, 0 `unsafe`.

Existing release diagnostics remain explicit performance debt: maintained I64 Group ~2.007x hand baseline; maintained I64 TopK ~5.175x. Semantic Join crossover still shows scan winning at 1k rows and indexed scaling winning by 10k/50k.

## rejected routes

- no durable/semantic index ownership;
- no unconditional auto-build rule;
- no exact-byte claim for the key-cell budget;
- no advisor eviction of externally installed indexes;
- no structural/custom fake canonicalization;
- no premature generic-I64-vs-specialized-I64 winner;
- no claim that explicit workload samples equal autonomous telemetry.

## recommended next step

Build the next planner layer around multiway/nested join-order enumeration and multi-family access costing. It should compare scan, existing generic semantic indexes, the specialized I64 family and prospective builds from common statistics. Keep exact memory accounting/autonomous telemetry and the I64 Group/TopK constant-factor tracks explicitly OPEN.
