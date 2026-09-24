# CFMD implementation report — Pass67

Authoritative base: verified Pass66. Corrected Program7 was not available and no Program7 code is present.

## Production changes

### 1. Advisor-owned semantic statistics for a verified direct-Join use case

`MaterializedSemanticStatisticsState` now participates in the physical lifecycle advisor for the conservative case where exact distinct-key cardinality changes a direct primitive Join from a transient-index plan to the cheaper scan.

The advisor:

- discovers only direct Join bindings whose current cost/executor path consumes the statistic;
- skips bindings already covered by exact persisted I64/semantic access paths;
- builds the candidate once, uses its exact distinct count to compute repeated-work savings and retained-byte cost, then publishes the selected state without rebuilding it;
- enforces managed/global byte budgets;
- evicts only advisor-owned statistics;
- treats explicit `install_semantic_statistics()` as a manual pin/ownership transfer;
- exposes equivalent wrappers through `RuntimeRevisionCell` and `DurableRuntime`, with one immutable-root publication only on physical change.

The scope is intentionally narrower than general autonomous statistics. Multiway plan-order counterfactuals, histograms/correlation and workload telemetry remain OPEN.

### 2. Persistent outer `PhysicalStore` catalog roots

The seven large outer physical directories/ownership set are now shared `Arc<BTreeMap/...>` or `Arc<BTreeSet/...>` roots. Private mutation helpers detach through `Arc::make_mut`.

Relation-delta maintenance first detects whether a family has an affected binding, so an untouched family does not detach merely because another relation artifact changes. This removes the Pass57 O(number-of-artifacts) catalog-metadata clone from `PhysicalStore::clone()` while retaining the existing per-artifact COW payload isolation.

### 3. Tests / hostile checks

New/strengthened production tests cover:

- duplicate-heavy Join: statistics advice changes `ephemeral_index_builds` from 1 to 0 without changing rows;
- budget rejection and manual pinning;
- no runtime republish on an unchanged second advice turn;
- no redundant statistics when an exact persisted Join access artifact already exists;
- pointer identity of all outer `PhysicalStore` catalog roots across clone and selective detach after relation delta.

Full debug/release workspace, strict Clippy, strict rustdoc and release overflow-check all pass on frozen source. Freeze and post-gate source hashes are identical.

## Performance evidence

The existing release clone diagnostic (200k-row relation, 50 clone samples) gave one matched Pass66/Pass67 observation:

- relation-only: 1128 ns → 42 ns average clone;
- with one persisted I64 index: 218 ns → 40 ns average clone.

The test is deliberately diagnostic and noisy. The production claim is structural O(1) root sharing with selective COW detach, directly checked by `Arc::ptr_eq`; the nanosecond values are not an SLA.

## Ledger result

Historical OPEN #9, persistent outer physical artifact catalogs, is CLOSED. The historical active count therefore moves from 24 to **23**.

The broader lifecycle item remains OPEN: statistics are managed only for the exact direct-Join case above, while QCN support, layouts, multiway counterfactual scoring, write-rate costing and autonomous telemetry remain unresolved.
