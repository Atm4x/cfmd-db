# CFMD implementation report — Pass66

Pass66 starts from verified Pass65 and contains no Program7 code.

## Production changes

### 1. Persistent/COW logical `DatabaseState`

`kernel-model` now stores large logical roots behind copy-on-write sharing:

- carriers and fields: `CowMap<...>`;
- lifecycle: `CowValue<LifecycleGraph>`;
- relation directory: COW map;
- each relation payload: independent `Arc<Vec<Row>>`.

`DatabaseState::clone()` therefore shares logical payloads. A mutation detaches only the relevant COW roots. `kernel-transport` and `storage-memory` were adapted to preserve ordinary value semantics at their boundaries.

A pointer-identity hostile proves source isolation and sharing of untouched roots. The previous 40×5,000-row clone diagnostic drops from a Pass65 median of 4.331 ms to tens/hundreds of nanoseconds for root clone / clone+relation-root replacement. This is not a claim that touched relation reconstruction or BTreeMap metadata path-copy is O(|Δ|).

### 2. Persisted I64 physical-artifact advisor

`kernel-plan` adds `advise_i64_indexes` at PhysicalStore, RuntimeRevisionCell and DurableRuntime layers plus `I64IndexAdvisorReport`.

Admission is deliberately narrow: only Join observations that map to the executor's persisted-I64 access path count as benefit. One-shot workloads are rejected before candidate build; repeated profitable joins may create an index; existing manual indexes can be reused but are never taken into advisor ownership; empty workload may evict advisor-owned indexes only. Pass63 managed/global retained-byte budgets apply.

An initial Filter-based admission path was removed during hostile review because Filter did not consume the resulting persisted I64 artifact.

## Changed production files

- `crates/kernel-model/src/lib.rs`
- `crates/kernel-plan/src/lib.rs`
- `crates/kernel-transport/src/lib.rs`
- `crates/storage-memory/src/lib.rs`

## Verification

All eight Rust 1.98.1 final gates pass on the frozen source. Freeze and post-gate SHA-256 snapshots of every file under `crates/` are byte-identical.

Current snapshot: 417 tests, 161 kernel-plan tests, 21 crates, 58,327 Rust LOC, 0 unsafe, 0 external Cargo sources, no new lint suppression.

## Ledger effect

Pass65 had 25 active OPEN: the corrected historical 24 plus the new full-`DatabaseState` clone problem. Pass66 closes that new problem, so the active ledger returns to **24**. The historical multi-family lifecycle item advances through a third managed physical family but remains OPEN.
