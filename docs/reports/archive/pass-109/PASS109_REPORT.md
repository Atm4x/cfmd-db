# CFMD Pass109 — Flat NodeId State Arena COMPLETE

## Status

- Versioned State V3: **COMPLETE**.
- ExecGraph V4 unified production planner/patch runtime: **COMPLETE**.
- V5 Stage 6: **COMPLETE**.
- Historical #21/#22: **PROD CLOSED**.
- Flat authoritative NodeId state arena: **COMPLETE**.
- Historical production closure remains **16 / 22**; flat arena was an engineering cleanup, not a separate historic problem.

## Problem → hypothesis → implementation → falsification → result

### Problem
Pass108 used NodeId scheduling but still retained maintained operator state as a recursive tree and rebuilt a temporary postorder state view for each transition. This left execution coordinates and physical ownership misaligned.

### Hypothesis
`PreparedRelGraph` already gives a stable postorder NodeId. The same NodeId can be the physical ownership coordinate for maintained state without changing any relational algebra, Delta ABI, GraphPatchSet semantics, Versioned State publication rule, or public API.

### Implementation
- Added flat maintained runtime node/state representation keyed directly by NodeId.
- Runtime state is owned as `Arc<Vec<Arc<FlatMaintainedRelPlanNode>>>` in compiled postorder.
- Child references inside runtime state are NodeIds, not recursive boxes.
- V4 planning reads maintained state directly by NodeId; transient `collect_postorder_states()` discovery is removed.
- Leaf validation, storage-resolved validation, scan discovery, output reconstruction and GraphPatchSet commit all address the same arena.
- Storage handle binding uses arena COW.
- Release builds drop the recursive construction tree immediately after flattening (`state.node == None`).
- Debug builds retain/synchronize the recursive representation only as the existing differential oracle.
- COW isolation remains explicit at the authoritative arena Arc boundary.

### Falsification
Final gates were rerun after the last release-only structural assertion:

- `cargo fmt --all -- --check`: **PASS**.
- `cargo clippy --workspace --all-targets -- -D warnings`: **PASS**.
- `cargo clippy --workspace --all-targets --release -- -D warnings`: **PASS**.
- release-only `release_maintained_plan_discards_construction_tree_after_flattening`: **PASS**.
- `cargo test --workspace --all-targets`: **698 passed / 0 failed / 8 ignored (706 declared)**.
- Earlier Pass109 kernel-query debug suite: **103 passed / 0 failed / 1 ignored**.
- Earlier Pass109 kernel-query release suite: **103 passed / 0 failed / 1 ignored**.
- Stage-6 smoke after arena cutover: linear Join→Group→TopK **12.749 us median**, Blocker→Group→TopK **10.496 us median**.
- Allocation probe: **138 median / 138 p90 / 143 max calls/update**, versus 139 median in Pass108.

### Result
**Flat NodeId Arena is COMPLETE.** Release runtime has one authoritative maintained-state layout: the NodeId arena. The old recursive layout is construction-only and is discarded after flattening; only debug/test builds retain a synchronized oracle copy for differential checking.

## Freeze

- UTC: 2026-09-23T14:07:05Z
- `crates/kernel-query/src/lib.rs` SHA-256: `af4180fce1c8d0eee3f705bfe4e50edb65c43542bfb5dcf877c2413620e1134a`
- `crates/kernel-query/src/execgraph.rs` SHA-256: `c0acc60172c906ce463a1ddb9d1b10684bd11409323ee3a07c9f20c46f14438d`
- No source change was made during final completion gates.

- Production fingerprint: `0272126e434151108fe3f4f0a686b242229c8b8608b5834094696988745d56f1`

## Next frontier
Pass110 should return to the historical ledger. Recommended order: **#18 first, then #16**. See `PASS110_FRONTIER_18_16.md`.
