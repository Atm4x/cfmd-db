# IMPLEMENTATION REPORT — Pass45

## problem

Pass44 had cost-gated Join families and contiguous multiway reassociation, but access-family choice was still duplicated across physical paths. Intermediate Join execution did not symmetrically implement every current selected transient family, optimistic pre-build distinctness could mislead the physical choice, and duplicate-heavy fused fallback could build the same transient I64 index more than once.

## hypotheses

1. Scan, persisted specialized I64, persisted generic semantic, transient I64 and transient generic semantic Join access are physical alternatives of one exact operation and should share one deterministic decision boundary.
2. A speculative transient estimate may guide whether to try a build, but actual built distinctness must be re-costed before the transient representation is committed as the execution path.
3. Multiway planning must not treat unknown transient distinctness as measured state; until richer statistics exist, persisted statistics or scan are the sound cost inputs.
4. Rejected transient state should fall to exact scan without escaping into a path that reconstructs the same transient index.

## implementation

- factored scan/persisted/ephemeral Join work calculations in `SemanticAccessCostModel`;
- added internal `JoinAccessFamily` / `JoinAccessDecision` / `RightJoinAccessRequest`;
- direct Join dispatch now chooses one family through the common selector;
- multiway persisted merge estimation consumes the same selector with transient speculation disabled;
- intermediate-left/direct-right Join execution now supports persisted I64, persisted semantic, transient I64, transient semantic and scan;
- transient intermediate access is re-costed after build using actual distinct-key count;
- `Join -> Project` selection consults the common access decision before specialized execution;
- typed/fused I64 batch execution uses a common family selection and an explicit in-path `FullScan` fallback after rejected transient build;
- preserved `persisted_index_cost_rejections` and transient-build observability.

## hostile falsification

- deterministic persisted I64 vs persisted semantic choice;
- persisted semantic reuse before transient I64 build;
- transient generic semantic family selection when appropriate;
- direct/multiway shared persisted decision;
- no optimistic transient distinctness credited to multiway DP;
- transient execution after an intermediate Join;
- duplicate-heavy post-build selectivity rejection;
- exactly one transient attempt before fused exact-scan fallback;
- full Pass44 multiway/logical-reference regressions remain green.

## verification/result

Rust 1.98.1 full fmt/check/debug/release/strict-Clippy/release-build/strict-rustdoc/overflow-release gate: PASS.

Metrics: 340 declared tests, 112 `kernel-plan`, 21 crates, 46,222 Rust LOC, 0 external Cargo sources, 0 `unsafe`, 0 TODO/FIXME/todo!/unimplemented!.

Production source window: exactly 20:00, 12:03:40–12:23:40 UTC. Source remained frozen throughout the final gate and packaging.

## rejected routes

- no sequential helper order as optimizer semantics;
- no invented `distinct = rows` fact in multiway costing;
- no transient index promoted to persisted authority;
- no hidden second transient rebuild after rejection;
- no claim that Join access unification closes multi-family lifecycle management or arbitrary bushy planning.

## historical ledger

Pass45 closes 3 concrete current-cycle problems. It fully closes **0 / 22** broad historical OPEN items from Pass44. **22** historical OPEN items remain. **0** new architectural OPEN items were created.

## recommended next step

Add retained cardinality/selectivity statistics and arbitrary leaf permutation/general bushy Join enumeration on top of `JoinAccessDecision`, then widen physical lifecycle ownership across specialized I64/generic/future layouts under one memory/benefit accounting model.
