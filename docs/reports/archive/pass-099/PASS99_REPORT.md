# PASS99 REPORT

**Status:** FINAL / V5 STAGE 4.3 ORDEREDBOUNDARY / TOPK COMPLETE

## Baseline and wall-clock boundary

Baseline: frozen Pass98 (`cfmd_workspace_pass98_topk_scalar_orderedboundary.zip`). Source work began at **2026-09-23 02:13:03 UTC**. Nominal 20-minute source boundary: **02:33:03 UTC**; hard 24-minute stop: **02:37:03 UTC**. Production source was frozen before the nominal boundary.

## Result

- **V5 Stage 4.3 OrderedBoundary / TopK — COMPLETE.**
- `I64Scalar`, `I64Rows`, and `SemanticOrdered` now share one read-only `DeltaView -> PlannedDeltaEffect<TopKDeltaPatch, AdaptiveDelta<Row,4>> -> commit` boundary.
- Historical production closure remains **14 / 22**. #21/#22 remain integration-in-progress until Stage 4.4/4.5, Stage 5, and Stage 6 gates are complete.
- Production source delta vs Pass98 is exactly one Rust file: `crates/kernel-query/src/lib.rs`.
- Frozen path-stable source fingerprint: `9e7645c1001bf32e04ad03717ac556923aa145396249127270ed9f699889434a`.
- Final gate: fmt/check/strict Clippy/full workspace tests PASS; **695 declared tests / 0 failed / 8 ignored**.

## Stage 4.3b integration

`MaterializedTopKDeltaState::apply_input_delta` no longer owns three mutation protocols. It validates the public compatibility delta once, plans through one universal `plan_delta_view`, materializes the universal effect only after planning succeeds, and then commits one typed `TopKDeltaPatch`.

The patch variants preserve replaceable physical state:

- scalar exact-I64: next `I64TopKState` from the Pass98 dense/radix planner;
- non-scalar I64 rows: checked per-key bucket replacement patch;
- generic Γ ordering: checked `IndexedOrderedMutationPlan` over semantic index identities.

Both migrated fallback backends produce `AdaptiveDelta<Row,4>` directly rather than mutating state and reconstructing a `RelationDelta` from whole `RelationValue` before/after snapshots. Generic fallback may still materialize selected-prefix row vectors while deriving the effect; Stage 6 owns performance/allocation closure, so this checkpoint does not claim fast-path parity for those uncommon physical shapes.

Signed I64-row input is normalized removals-before-inserts inside each affected order bucket. Consequently planning is independent of the visitation order chosen by a concrete `DeltaView` carrier.

## Hostile falsification

- non-scalar I64-row TopK is forced onto `MaintainedTopKStorage::I64Rows`, moved across a WITH-TIES boundary, and compared with full recomputation;
- a missing I64-row removal fails before commit and leaves complete maintained state unchanged;
- generic case-insensitive text ordering remains on `SemanticOrdered` and preserves semantic ties;
- a mixed malformed generic delta containing a valid insertion plus missing removal fails atomically with no reserved-id/state leak;
- all pre-existing scalar promotion/fallback, 2,000-step threshold, Bag/Set, ascending/descending, semantic-ordering and Join->Group->TopK tests remain green.

## Deliberate non-claims

- Stage 4.4 BilinearPullback / Join is not integrated in this checkpoint;
- Stage 4.5 BlockerZeroCrossing is not integrated;
- Stage 5 root-only compatibility materialization is not started;
- generic `I64Rows` / `SemanticOrdered` planning still prioritizes correctness/one kernel contract over final allocation/performance lowering;
- no Stage 6 benchmark closure and no PROD CLOSED claim for #21/#22.

## Next

Pass100 should migrate Join as Stage 4.4 BilinearPullback under the same read-only plan/commit + universal-effect contract. Then Stage 4.5 Blocker can use the separately supplied converged execution-graph R&D only after rebasing its claims onto the current production tree.
