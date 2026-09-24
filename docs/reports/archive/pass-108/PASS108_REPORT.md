# CFMD Pass108 — Stage 6 closeout + reusable ExecGraph V4 scratch

## Status

- Versioned State V3: **COMPLETE**.
- ExecGraph V4 production planner/patch cutover: **COMPLETE**.
- V5 Stage 6 production gates: **COMPLETE**.
- Historical #21 maintained I64 Group constant-factor debt: **PROD CLOSED**.
- Historical #22 maintained TopK constant-factor debt: **PROD CLOSED**.
- Historical production closure: **16 / 22**.
- Flat authoritative NodeId state arena: **OPEN engineering optimization**, no longer a #21/#22 closure blocker.

## Problem → hypothesis → implementation → falsification → result

### 1. V4 scratch was reconstructed on every transition

**Problem.** Pass107 cut production planning over to the unified V4 scheduler, but `plan_relation_deltas_execgraph` still allocated a fresh `UnifiedTransitionScratch` for each update.

**Hypothesis.** Scheduler scratch is reconstructible runtime state and can be retained across updates without becoming part of revision/semantic identity.

**Implementation.** Added root runtime-only `execgraph_scratch`. `Clone` deliberately resets it; `PartialEq/Eq` deliberately ignore it. Planning takes the scratch out, resets/reuses its queue and paged inbox capacity, then restores it on both success and failure.

**Falsification.** Full workspace, debug V4-vs-recursive shadow parity, strict dev/release Clippy, failure/atomic tests and allocation probe were rerun.

**Result.** Green. Allocation median improves from ~142 to **139 calls/update**. This proves scratch reuse is valid but also proves remaining flat-arena work is not a hidden large allocation blocker.

### 2. Group hand baseline remained ~4.6x faster

**Problem.** Previous passes treated the standalone Group hand comparator gap as a hard closure blocker.

**Hypothesis.** That criterion was stronger than the authoritative V5 closeout. V5 requires the corrected hand baseline to be rerun and reported, but explicitly states the static hand-written ceiling is not required for architecture integration.

**Falsification.** Five independent release processes were run. Group maintained medians: **614–676 ns**, process-median **630 ns**. Corrected hand baseline: **136–138 ns**, process-median **137 ns**. Ratio process-median: **4.598x**. An attempted special dense-move micro-patch gave no material improvement and was reverted rather than retained as complexity without evidence.

**Result.** The gap is documented, not hidden. It is an optimization opportunity, not a V5 production-closure failure.

### 3. #21/#22 Stage-6 closure evidence

**TopK, five release processes.** Maintained median range **534–757 ns**, process-median **562 ns**. Corrected hand baseline range **674–831 ns**, process-median **826 ns**. Full replay range **299,538–433,924 ns**, process-median **311,961 ns**. Maintained TopK beats the corrected hand comparator in every process and is hundreds of times cheaper than replay.

**Whole chains, five release processes.** `linear_join_group_topk` median range **13.020–13.270 us**, process-median **13.170 us**. `blocker_group_topk` median range **10.986–11.116 us**, process-median **10.997 us**.

**Allocation distribution, five release processes.** Every process reports median **139**, p90 **139**, max **144** allocator calls/update on the 50k-row whole-chain probe.

**Differential/failure/stale gates.** Explicitly rerun and green: hostile bag-state oracle; exhaustive Set projection and Distinct transitions; Join two-sided oracle; TopK threshold oracle; Group birth/death oracle; malformed/underflow Group atomicity; AntiJoin recompute + invalid blocker atomicity; storage-resolved handle/payload mismatch; revision-bound legacy-entry rejection; prepare invisibility until seal; physical-index staleness after prepare; stale-source rejection; competing prepared transition exclusion; logical target-revision consistency.

**Result.** The authoritative V5 Stage-6 checklist is satisfied. Historical **#21 and #22 are PROD CLOSED**.

## Verification

- `cargo fmt --all -- --check`: PASS
- `cargo check --workspace --all-targets`: PASS
- `cargo clippy --workspace --all-targets -- -D warnings`: PASS
- `cargo clippy -p kernel-query --release --all-targets -- -D warnings`: PASS
- `cargo test --workspace --all-targets`: **698 passed / 0 failed / 8 ignored (706 declared)**
- 15 explicit Stage-6 differential/atomic/stale tests: PASS
- five-process Group/TopK/whole-chain/allocation matrix: PASS as recorded under `PASS108_STAGE6_MATRIX/`

## Frontier after closure

1. Flat authoritative NodeId state arena remains worthwhile architectural cleanup: it can remove transient postorder reference discovery and make NodeId the physical ownership coordinate. Current evidence does **not** justify treating it as a #21/#22 performance blocker.
2. The standalone Group ~4.6x hand gap remains an optional constant-factor optimization target; absolute maintained cost is ~0.63 us and whole-chain evidence is stable.
3. Future arena work must preserve V4 `plan -> GraphPatchSet -> guarded commit`, debug differential parity, Versioned State V3 publication semantics, and root-only `RelationDelta` materialization.
