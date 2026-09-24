# CFMD implementation report — through Pass19

Current verified checkpoint: Pass19, 2026-09-19.
Measured implementation/audit cycle: 20m11s (14:24:58–14:45:09 UTC).

## Executive state

The logical kernel remains unchanged: `Revision = (Schema S, SemanticEnv Γ, finite Model M)`. Pass19 changes physical execution/storage representation only.

The implementation now has:

- algebraic structural values plus nominal entity identity;
- explicit Set/Bag/Seq/Map semantics under versioned semantic modules;
- guarded μ equivalence;
- exact closed relational query IR and exact recomputation derivative fallback;
- optimized/operator-local relational deltas for the current `RelExpr` set;
- long-lived Set/Distinct derivative support state;
- lifecycle least-fixed-point normalization;
- identity transport fragment and revision DAG merge machinery;
- checked certificates and typed PlanIR;
- row + typed heterogeneous columnar execution;
- physical Scan/Filter/Project/Distinct/Join/Group/TopK/PromoteToBag correctness paths;
- near-baseline typed Scan→Filter→Project microkernel;
- adaptive/persisted I64 Join;
- atomic relation+index delta maintenance;
- index-assisted semantic row removal lookup;
- generational reusable physical row slots with bounded lifetime metadata;
- first fused indexed Join→Project path that avoids complete intermediate joined rows.

## Pass19 implementation changes

### Generational physical handles

`PhysicalRowId` is now `(slot,generation)`. Deleted slots are reusable; reuse increments the generation. A stale handle fails resolution rather than aliasing a later row.

`InstalledRelation` now stores dense physical rows plus a generational slot table, free-list, and O(1) logical-order links. Dense deletion remains `swap_remove`, updating at most one moved-row position. Logical scan order is independent of physical order.

10,000 delete/insert cycles on a one-row relation retain exactly one slot while advancing generation to 10,000. Generation overflow is prevalidated before mutation, preserving atomicity even at the handle-generation boundary.

### Index rebuild ordering

`MaterializedI64IndexState::build` follows relation logical scan order. A fresh index rebuilt after compaction/churn therefore preserves the same right-row ordering as the maintained index/reference semantics.

### Zero-copy free-list planning

`planned_insert_ids` no longer clones the free-list. It previews the exact LIFO reuse order from same-transition removed slots, existing free slots, then fresh slots.

### Join→Project fusion

A `Project` directly above an adaptive indexed I64 Join can now execute as a fused probe/projection kernel. Only requested final columns are materialized. Persisted and ephemeral index variants preserve the old exact fallback behavior.

## Verification

Pass19: 200 declared tests, all debug/release PASS. Strict fmt, Clippy `-D warnings`, release build, strict rustdoc and release overflow-check plan/integration tests all PASS.

19 crates; 20,901 Rust LOC; zero external Cargo sources; zero `unsafe`; zero TODO/FIXME; zero panic-shaped production/test macros found by audit.

## Current physical performance findings

- indexed removal at 300k rows remains around 10–12 µs versus ~16 ms linear semantic scan for an unindexed last-row removal;
- persisted 20k-key I64 Join remains roughly 1.15–1.22x a hand-written prebuilt BTreeMap baseline in current repeated runs;
- persisted execution remains roughly half the time of rebuilding the index per query.

These are diagnostic microbenchmarks, not universal DB performance claims.

## Immediate implementation frontier

1. general typed batch/row-handle DAG across arbitrary operator trees;
2. broader persisted semantic key indexes and multi-index candidate/cost planning;
3. measure whether historical-peak free-slot capacity needs chunk/epoch reclamation;
4. stateful TopK and Group;
5. remaining physical layout families and OrderedView;
6. durability/recovery/crash testing;
7. transaction-repair runtime, distribution, formal closure.

See `CFMD_IDEAL_DB_SPEC.md` for the normative target and `PASS19_REPORT.md` for the detailed problem→hypothesis→implementation→falsification→result chain.
