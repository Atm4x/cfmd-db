# PASS97 REPORT

**Status:** FINAL / V5 STAGE 4.2 ANNOTATION-GROUP INTEGRATED

## Baseline and wall-clock boundary

Baseline: frozen Pass96 (`cfmd_workspace_pass96_zero_crossing_barrier_kernel.zip`). Source work began at approximately **2026-09-23 01:16:59 UTC**. The nominal 20-minute source boundary was **01:36:59 UTC**. Production source was frozen early at **01:34:27 UTC**. After freeze no `.rs` file or Cargo manifest was changed.

## Result

- **V5 Stage 4.2 Annotation / Group — COMPLETE.**
- Historical production closure remains **14 / 22**. #21/#22 remain integration-in-progress, not closed.
- Source delta vs Pass96 is exactly one Rust file: `crates/kernel-query/src/lib.rs`.
- Frozen path-stable source fingerprint: `496562b3298022c3e69b244aa278bceca3da01c39b37e7053705136a83185346`.
- Final gate: fmt/check/strict Clippy/full workspace tests PASS; **691 declared tests / 0 failed / 8 ignored**.

## Stage 4.2 integration

`MaterializedGroupDeltaState` is now a genuine two-phase barrier kernel. Both generic Γ-aware Group and exact-I64 Count consume a universal `DeltaView<Row>`, produce a universal `AdaptiveDelta<Row, 4>`, and keep mutation in an explicit commit step. The legacy public `RelationDelta` boundary is retained only as compatibility materialization.

The exact-I64 Count lowering now has a v3-style `DenseWindowGroupCount` physical tier backed by `ExactCount` cells. Admission uses physical span/memory limits (`margin=64`, `max_slots=4096`). An out-of-window delta plans sparse fallback before commit rather than failing semantically. The sparse mirror remains exact and authoritative for fallback.

The dense replacement microkernel detects `-1 source / +1 vacant-target` where source multiplicity is exactly one and moves the existing `ExactCount` object with `mem::take`; it does not decrement/recreate the dense cell value.

## Hostile falsification

- read-only dense planning leaves state byte-for-byte unchanged before commit;
- dense replacement matches full recomputation;
- an outlier key (`10_000`) leaves planning successful, selects sparse fallback, and still matches recomputation after commit;
- generic Group sequential Count/ExactF64Sum and atomic-underflow tests remain green under the new plan/commit path;
- the first move-path test initially exposed a test-fixture mistake (target was not vacant); corrected hostile explicitly requires and observes the move path;
- full workspace regression remains clean.

## Deliberate non-claims

- Stage 4.3 OrderedBoundary/TopK is **not started**. Current TopK still requires the v3 DenseUnit/DenseCounted/PagedRadix promotion/fallback integration plus universal plan/commit conversion.
- Join and blocker classes are not migrated yet.
- Stage 5 root-only `RelationDelta` materialization is not started.
- No benchmark closure or PROD CLOSED claim is made for #21/#22.

## Next

Pass98 should implement Stage 4.3 OrderedBoundary/TopK as one whole checkpoint: v3 physical tiering with fail-atomic promotion/fallback, universal `DeltaView -> plan/commit -> AdaptiveDelta`, differential oracle for ascending/descending/ties/Bag/Set, then full workspace gate. Only after that should Stage 4.4 Join begin.
