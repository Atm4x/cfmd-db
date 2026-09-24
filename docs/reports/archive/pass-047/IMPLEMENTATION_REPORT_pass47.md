# IMPLEMENTATION REPORT — Pass47

## problem

Pass46 showed that arbitrary Join leaf permutation was not safe under the current exact physical contract: logical `JoinEq` produces deterministic left-major/right-minor Bag order, while a reordered physical tree can emit the same tuples in a different observable sequence and column layout.

## hypotheses

1. Original base-leaf logical scan ordinals are sufficient physical provenance to reconstruct the exact row-production order of the current flattened equality-Join fragment.
2. Reordered intermediate rows can carry per-leaf fragments without promoting that provenance to semantic authority.
3. A three-way non-contiguous permutation is safe only if final column order and row order are restored before the physical result is returned.
4. Restoration work must be part of the cost decision rather than treated as free.

## implementation

- added `ExecutionStats.multiway_join_order_restorations`;
- added correctness-first `ProvenanceJoinRow` carrying original-leaf row fragments and logical scan ordinals;
- base provenance is derived from authoritative `PhysicalStore::logical_rows_with_handles` scan order;
- reordered merges evaluate the existing exact pinned-Γ equality predicates;
- final restoration sorts lexicographically by original leaf ordinals and flattens fragments in original leaf order;
- added retained-statistics cardinality helpers for candidate pair/third-join estimates;
- added explicit estimated restoration-sort work;
- added the first admitted non-contiguous three-way primitive equality path, choosing `(leaf0, leaf2)` first only when complete estimated work beats adjacent alternatives;
- the initial permutation path deliberately declines when relevant persisted semantic/I64 indexes exist, leaving established indexed execution unchanged.

## hostile falsification

- synthetic fragments prove restoration follows original scan ordinals, not reordered physical join order;
- real three-relation Bag result equals logical reference exactly after a non-contiguous first join;
- existing persisted-index multiway path is not intercepted;
- missing-retained-statistics path is not intercepted;
- restoration cost participates in plan admission;
- debug and release suites agree;
- no new lint suppression, unsafe code or assertion-only mutation introduced.

## verification/result

Full Rust 1.98.1 fmt/check/debug/release/strict-Clippy/release-build/strict-rustdoc/overflow-release gate: PASS after source freeze.

Metrics: 349 declared tests, 121 `kernel-plan` tests (120 normal + 1 ignored diagnostic benchmark), 21 crates, 47,310 Rust LOC, 0 external Cargo sources, 0 `unsafe`.

## rejected routes

- no treating equality Join as freely commutative under an observable Bag row-order contract;
- no general N-way claim from one three-way fragment;
- no optimizer that omits restoration cost;
- no new logical provenance primitive;
- no host `Eq`/`Ord` substitution for Γ predicates;
- no lint suppression to hide the new planner helpers.

## recommended next step

Generalize the proven order-restored mechanism to N-way subset/bushy enumeration, but first/alongside replace logical `Row` fragment provenance with compact stable-handle/typed-native provenance and integrate existing persisted/transient `JoinAccessDecision` families into permuted execution.

## remaining risks

The broad multiway item remains OPEN. The current permutation path is intentionally three-way, retained-statistics-driven, non-indexed and correctness-first; N-way search, indexed permuted plans, richer predicate graphs and lower provenance/restoration materialization cost remain future work.
