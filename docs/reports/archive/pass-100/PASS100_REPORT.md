# PASS100 REPORT

**Status:** FINAL / V5 STAGE 4.4 BILINEAR PULLBACK / JOIN COMPLETE

## Baseline and wall-clock boundary

Baseline: frozen Pass99 (`cfmd_workspace_pass99_topk_orderedboundary_complete.zip`). Source work began at **2026-09-23 02:38:49 UTC**. Nominal 20-minute source boundary: **02:58:49 UTC**; hard 24-minute stop: **03:02:49 UTC**. Production source was frozen before the nominal boundary.

## Result

- **V5 Stage 4.4 BilinearPullback / Join — COMPLETE.**
- `I64`, primitive-Γ `SemanticIndexed`, and structural-Γ `StructuralIndexed` Join backends now share one read-only binary `DeltaView × DeltaView -> PlannedDeltaEffect<JoinDeltaPatch, AdaptiveDelta<Row,4>> -> commit` boundary.
- Join output is emitted directly from the bilinear differential identity `ΔL ⋈ R_old + L_new ⋈ ΔR`; no intermediate owned `RelationDelta` is required inside the barrier planner.
- The right-side term is evaluated against the planned left state, so the `ΔL ⋈ ΔR` cross-term is represented exactly once.
- Public compatibility still materializes one `RelationDelta` only after both side plans and the output effect succeed, then commits the typed patch.
- Historical production closure remains **14 / 22**. #21/#22 remain integration-in-progress until Stage 4.5, Stage 5 and Stage 6 gates complete.
- Production source delta vs Pass99 is exactly one Rust file: `crates/kernel-query/src/lib.rs`.
- Frozen path-stable source fingerprint: `d21904012350dd116225d73e3e1c587287504be2926b3396a1f7dff26b7cb03b`.
- Full gate: fmt/check/strict Clippy/full workspace all-target tests PASS; **696 declared tests / 0 failed / 8 ignored**.

## Stage 4.4 integration

`MaterializedJoinDeltaState::apply_input_deltas` is now only a public compatibility wrapper. It checks the externally carried relation types, calls one binary read-only planner through zero-copy `RelationDeltaView`s, materializes the certified adaptive effect, and commits only after planning succeeds.

The physical patch variants are:

- exact-I64: changed bucket replacements for left and right;
- primitive Γ: checked semantic-index identity removal/insertion plans for both sides;
- structural Γ: checked canonical structural-index identity removal/insertion plans for both sides.

All side planners normalize signed carrier semantics rather than inheriting a concrete carrier's visitation order. Weighted Bag entries (`|weight| > 1`) are accepted without first expanding an owned compatibility `RelationDelta`; Set uniqueness and removal underflow remain fail-closed.

The output planner is deliberately independent of mutation planning. Once both patches are known valid, output is derived as:

`left_delta against old right` + `right_delta against planned left`.

This is the existing correct Join algebra expressed directly in the universal Delta ABI, not a new Join algorithm.

## Hostile falsification

- existing two-sided exact-I64 Join vs full recompute remains green;
- direct `AdaptiveDelta` hostile inserts multiplicity `+2` on the left while changing the right in the same transition and matches full recomputation, exercising the bilinear cross-term;
- the same hostile verifies authoritative Join state is unchanged after planning and changes only after explicit commit;
- valid left plan + invalid right removal fails the complete binary plan without committing either side;
- primitive case-insensitive Γ Join remains on `SemanticIndexed` and preserves semantic equality;
- same-call remove/insert replacement remains legal;
- structural Γ Join remains on canonical structural indexing and matches recomputation;
- recursive Join→Group→TopK and maintained-plan composition tests remain green.

## Deliberate non-claims

- Stage 4.5 BlockerZeroCrossing is not integrated in this checkpoint;
- Stage 5 root/public/persistence-only `RelationDelta` materialization is not started;
- existing compatibility callers still pass `RelationDelta` between some maintained-plan nodes; Stage 5 owns removal of those internal materializations;
- no Stage 6 corrected performance/allocation closure and no PROD CLOSED claim for #21/#22;
- the supplied `CFMD_RND_EXECGRAPH_BLOCKERS_V3_CONVERGED_2026-09-23.zip` remains an R&D oracle for the next Blocker/post-Stage5 work and was not mechanically merged.

## Next

Pass101 should integrate **Stage 4.5 BlockerZeroCrossing** against frozen Pass100. After all Stage-4 barriers are on the certified ABI, begin Stage 5 and remove internal compatibility `RelationDelta` construction except at root/public/persistence boundaries.
