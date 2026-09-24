# CFMD Pass45 Report — First-Class Multi-Family Join Access Decisions

**Status:** VERIFIED on Rust 1.98.1.

**Scope:** physical Join planner/executor data-plane only. Pass45 does not change semantic authority, durable revision authority, Γ laws, transaction publication law, or Pass43 advisor ownership.

**Production source window:** 2026-09-20 12:03:40 UTC → 12:23:40 UTC (**20:00 exactly**). After the deadline production source was frozen. Only verification, evidence, documentation, manifest and packaging followed. The freeze was not lifted.

## 1. Problem

Pass44 established cost gating and the first contiguous multiway reassociation slice, but hostile review found three connected physical-planner defects:

1. Join access-family selection still existed as family-specific control flow in several direct/multiway/fused execution paths rather than one deterministic decision boundary;
2. a multiway/intermediate Join could estimate or request a transient family that the intermediate-left/direct-right executor did not symmetrically implement, and speculative transient distinctness could be over-optimistic before the index was actually built;
3. in the fused I64 batch path, a transient index could be built, rejected after seeing its actual duplicate-heavy selectivity, and then be built again by a fallback path.

## 2. One Join access-candidate decision

Pass45 introduces one internal `JoinAccessDecision` over the current primitive Join families:

```text
FullScan
PersistedI64
PersistedSemantic
EphemeralI64
EphemeralSemantic
```

The decision records estimated work, estimated output rows and whether an installed persisted candidate was rejected by costing. `SemanticAccessCostModel` now exposes explicit scan, persisted-probe and ephemeral-build+probe work functions used by this selector.

Tie behavior is deterministic. Equal work does not accidentally depend on whichever family-specific helper happened to run first. Scan wins an exact tie; persisted specialized/generic candidates are preferred before constructing transient state according to the explicit tie order.

The same right-scan decision boundary is now consumed by:

- ordinary direct primitive Join execution;
- contiguous multiway merge estimation for persisted candidates;
- intermediate-left / direct-right Join execution;
- `Join -> Project` family selection;
- the current typed/fused I64 Join batch path.

This is physical planning only. Every indexed candidate remains derivative state and exact Γ equality remains the semantic authority.

## 3. Intermediate Join execution matches the candidate model

A later base relation can now be probed after an intermediate left Join using whichever current primitive family the unified selector chose:

- persisted specialized I64;
- persisted canonical semantic index;
- transient specialized I64;
- transient canonical semantic index;
- exact scan.

For transient families the pre-build estimate deliberately assumes only a prospective model. After the physical index is built, execution recomputes the decision with its **actual distinct-key count**. If duplicates make build+probe unprofitable, the transient state is discarded as a candidate and exact scan is used.

The multiway dynamic-programming cost search is more conservative: it currently does **not** credit an unbuilt transient index using the optimistic `distinct = row_count` assumption. It credits persisted statistics or scan. This avoids a physical estimate becoming a false planner fact. Richer retained statistics remain OPEN.

## 4. Single-build rejection in fused I64 batch execution

Hostile duplicate-heavy testing found that post-build rejection of a transient I64 index could return to a fallback path which then attempted another transient build.

The batch path now has an explicit `FullScan` execution choice alongside persisted and ephemeral index state. A speculative ephemeral build is observable once in `ExecutionStats.ephemeral_index_builds`; if actual distinctness rejects it, the already-prepared batch program performs exact right-side scan directly instead of escaping and rebuilding the same derivative structure.

This closes the repeated-build defect without making the transient index authoritative.

## 5. Hostile / regression evidence

Pass45 specifically verifies:

- persisted I64 and persisted generic semantic candidates enter one deterministic family decision;
- persisted semantic is reused rather than constructing transient I64 when its cost wins;
- no persisted index allows either transient I64 or transient generic semantic selection when profitable;
- direct and multiway planning share the same persisted right-side access decision;
- multiway costing refuses to credit unknown transient distinctness as if it were measured statistics;
- a multiway Join can execute transient access after an intermediate result;
- duplicate-heavy intermediate transient access is re-costed after actual build and falls back exactly when unprofitable;
- duplicate-heavy fused `Join -> Project` attempts the transient build once, then performs exact scan rather than rebuilding;
- all current logical-reference and Pass44 reassociation regressions remain green.

Targeted `kernel-plan` result at freeze: **111 passed, 0 failed, 1 ignored diagnostic benchmark**.

## 6. Verification

Final Rust 1.98.1 gate after source freeze:

```text
cargo fmt --all -- --check                                  PASS
cargo check --workspace --all-targets                       PASS
cargo test --workspace --all-targets                        PASS
cargo clippy --workspace --all-targets -- -D warnings       PASS
cargo test --workspace --all-targets --release              PASS
cargo build --workspace --release                           PASS
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps  PASS
RUSTFLAGS='-C overflow-checks=yes' cargo test ... --release PASS
```

The first combined release attempt and the first overflow-release attempt hit external command timeouts during compilation. Per project rule they were recorded as neither PASS nor FAIL; each command was rerun separately on a warmed target and completed PASS.

Workspace metrics at freeze:

- 340 declared Rust tests;
- 112 `kernel-plan` tests (111 normal + 1 ignored Pass44 diagnostic benchmark);
- 21 crates;
- 46,222 Rust LOC under `crates/`;
- 0 external Cargo registry/git sources;
- 0 `unsafe` in `crates/`;
- 0 TODO/FIXME/todo!/unimplemented! in `crates/`.

## 7. Problem ledger

### CLOSED exactly in Pass45

1. ✅ **Current primitive Join families did not have one deterministic access-candidate decision shared by direct, multiway and fused execution.** `JoinAccessDecision` now compares scan, persisted specialized I64, persisted generic semantic, transient specialized I64 and transient generic semantic work through one physical boundary.
2. ✅ **Intermediate Join execution and speculative transient costing could disagree.** Intermediate-left/direct-right execution now implements the selected current families, and transient candidates are re-costed after build using actual distinctness; multiway planning does not treat optimistic unbuilt distinctness as measured statistics.
3. ✅ **Duplicate-heavy fused transient-I64 rejection could rebuild the same transient index on fallback.** The batch path now falls to exact scan in-place after one observable transient build attempt.

### Historical OPEN from Pass44 fully closed this pass

**0 / 22.** Pass45 materially advances the broad multi-family planner item, but does not close its lifecycle/future-layout scope. General multiway Join planning also remains broader than the current contiguous/in-order fragment.

### Advanced but still OPEN

1. 🟨 **First-class multi-family physical advisor.** Join execution/planning now has a common current-family candidate layer, but Pass43 lifecycle ownership still manages only generic semantic indexes; specialized I64/future layout creation-retention-eviction are not yet one advisor domain.
2. 🟨 **General multiway/bushy Join planning.** Current interval search preserves leaf order; arbitrary permutation, disconnected/general predicate graphs and general bushy search remain OPEN.
3. 🟨 **Statistics.** The planner has row counts and persisted distinct-key counts. It deliberately refuses to invent transient distinctness; retained histograms/correlation/multi-column cardinality statistics remain OPEN.
4. 🟨 **I64 Group/TopK constant factors.** Existing residual performance debt remains unchanged by this pass.

### Historical / active OPEN after Pass45 — 22

1. ⬜ Structural/custom-equivalence canonical indexing and typed production.
2. ⬜ General nested/multiway/bushy Join planning: arbitrary leaf permutation/non-contiguous plans/richer predicate graphs.
3. ⬜ First-class multi-family physical access advisor/lifecycle across specialized I64, generic semantic indexes and future layouts.
4. ⬜ Autonomous workload statistics/telemetry, retention decay and lifecycle scheduling.
5. ⬜ Exact physical-index byte/resident-memory budgeting and rebuild scheduling.
6. ⬜ Canonical-key encoding/version migration for long-lived physical caches.
7. ⬜ Remaining physical layouts and explicit `OrderedView`/pagination.
8. ⬜ Secondary-index / alternate-layout rebuild performance after recovery.
9. ⬜ Clone-heavy runtime transaction candidates → COW/persistent roots.
10. ⬜ Durable revision DAG / branch+merge ancestry.
11. ⬜ General historical durable-format migration framework.
12. ⬜ Arbitrary/plugin semantic executable artifact packaging/signing/deployment.
13. ⬜ Transaction intent/outcome retention + GC.
14. ⬜ Streaming/chunked checkpoints/metadata.
15. ⬜ Real machine power-loss assurance and Windows/network-FS/FUSE durability semantics.
16. ⬜ General lock-poison/restart policy.
17. ⬜ Group commit / async durability.
18. ⬜ Replication / consensus and broader distribution architecture.
19. ⬜ Durable-store authentication/MAC; current CRC32C is corruption detection only.
20. ⬜ Formal power-loss proof for rename/fsync/GC protocol.
21. ⬜ Transaction repair runtime.
22. ⬜ Formal mechanization.

### New OPEN created in Pass45

**0.** Hostile work refined existing optimizer/statistics debt; it did not create a new architectural requirement.

## 8. Rejected / deliberately deferred routes

- no sequential family-specific “try this, then that” ordering as the optimizer contract;
- no assumption that a transient index has one distinct key per row merely because that makes the plan look cheap;
- no persistence/installation of execution-local transient indexes;
- no claiming that unified Join execution also closes specialized-I64 lifecycle ownership;
- no arbitrary join permutation before column/predicate remapping and statistics are sufficient;
- no new Clippy suppression to mask Pass45 planner complexity.

## 9. Result / next direction

Pass45 gives current primitive equality Join execution a coherent multi-family access decision and removes an important speculative-build/fallback inconsistency.

The next highest-value planner cluster is **statistics + arbitrary leaf permutation/general bushy Join enumeration**, using the new candidate interface rather than duplicating index-family logic. In parallel, the Pass43 lifecycle advisor can be widened to own specialized I64/future physical families only after one common memory/benefit accounting contract exists.
