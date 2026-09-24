# PASS81 CHECKPOINT M — RELATIONAL WRITABLE COMPILER FOUNDATION

Status: **INTEGRATED / DISTRIBUTED DEBUG+CLIPPY VERIFIED / PROD PARTIAL**.
Base: verified Pass81 Checkpoint L runtime schema-migration bridge.
R&D authority: bundled write-integration closeouts, especially writable-view synthesis / rewrite-law inference / migration complements and the unified integration spine.

## Problem
The scalar `WritableViewPlan` compiler had no relational IR. `Project`, `Filter`, and `Join` therefore had no fail-closed way to express which source relation owns a write, which output coordinates came from that owner, or which determinant/guard/invariant facts must be discharged before a writeback can be admitted.

## Hypothesis
Compile relational writability into an explicit owner-side plan rather than a boolean property of `RelExpr`. Preserve semantic column provenance through the tree and emit proof obligations whenever a unique preimage is not statically established. Never use row position, host equality, hashes, or endpoint coincidence as authority.

## Implementation
- Added validated `RelWritableColumnBindings`: relation columns are bound to revision-local observables only when the observable's pinned equivalence exactly matches the schema column equivalence.
- Added `RelColumnOrigin`, `RelRewriteLiftStage`, `RelWritableObligation`, `RelWritableViewPlan`, and a three-way result: `Writable`, `Conditional { obligations }`, or `ReadOnly`.
- `Scan(owner)` is the only unconditional relational case.
- `Project` tracks visible/hidden owner coordinates, rejects duplicate columns, and emits hidden-complement / insertion-constructor obligations unless APNF determinant closure discharges them; projection semantic-collapse safety remains explicit.
- `FilterEqConst` / `FilterEqColumns` preserve owner provenance but require predicate admissibility, DTC guard non-impact, and VMF invariant closure.
- `JoinEq` is admitted only when exactly one side contains the owner. The other side is lookup-only; APNF determinant closure may discharge lookup uniqueness, otherwise `JoinLookupDeterminant` remains explicit. Owner-on-both-sides fails closed.
- `Distinct`, `Group`, `TopKWithTies`, `Difference`, `AntiJoin`, and `PromoteToBag` remain read-only until an explicit action policy/certificate exists.

## Falsification / hostile coverage
- owner Scan compiles unconditionally;
- Project+Filter exposes hidden-complement and dynamic guard obligations;
- owner-side Join never guesses a lookup preimage;
- Join with the owner on both sides is rejected;
- Distinct is rejected without action policy;

## Verification
Compilation/testing was deliberately distributed by package because the tool-call budget cannot sustain a cold monolithic workspace invocation.

- final `cargo fmt --all -- --check`: PASS;
- final `cargo check -p kernel-lens --all-targets`: PASS;
- `cargo clippy -p kernel-lens --all-targets -- -D warnings`: PASS;
- `cargo test -p kernel-lens --all-targets`: **14 passed / 0 failed / 0 ignored**;
- distributed `cargo check` and Clippy `-D warnings` across all 23 workspace crates: PASS;
- distributed debug tests across all workspace crates: **554 passed / 0 failed / 8 ignored** (**562 declared/run**).

No lint suppression was added. No monolithic workspace build/test was attempted.

## What M closes
M closes the missing **relational writable analysis/compiler IR** for owner-side Scan/Project/Filter/Join and provides the APNF determinant hook plus fail-closed proof-obligation surface needed by the next runtime layer.

## Still OPEN after M
1. executable `view FineChange -> unique owner RelationDelta/PreparedRewrite` synthesis, including the 0/1/>1 preimage rule;
2. direct planner/APNF query-local observable handoff into the compiler rather than caller-supplied bindings;
3. DTC-backed dynamic guard-certificate discharge;
4. VMF invariant-closure certificate discharge;
5. publication of the uniquely lifted relational Rewrite through the existing H/I durable Rewrite boundary;
6. REIC consumption plus residual/cube/coherence certificates;
7. historical restore and complement GC/Forget enforcement at query/API level.

The write kernel is materially advanced but **not claimed complete** at Checkpoint M.
