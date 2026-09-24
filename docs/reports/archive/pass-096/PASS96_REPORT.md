# PASS96 REPORT

**Status:** FINAL / V5 STAGE 4 FRAMEWORK + ZERO-CROSSING KERNEL INTEGRATED

## Baseline and wall-clock boundary

Baseline: frozen Pass95 (`cfmd_workspace_pass95_delta_kernel_stage2_3.zip`). Source work started at **2026-09-23 00:52:03 UTC**. Production source was explicitly frozen at **01:07:52 UTC** (~15m49s), before the 20-minute source boundary. After freeze no `.rs` file or Cargo manifest is changed; only reports, manifest and packaging are produced.

## Result

- V5 Stage 4 physical-program boundary is now explicit: `CompiledDeltaProgram` records deterministic `BarrierKernelClass` entries for every current non-linear DTC class.
- **Stage 4.1 ZeroCrossing is complete** for `Distinct` / set `Project` maintained state.
- `MaterializedSetSupportState` is now a real two-phase kernel: read-only `plan_delta_view` derives a checked patch plus universal signed output effect; `commit_support_patch` applies only the already-validated patch.
- `Distinct` consumes the zero-copy `RelationDeltaView`; `ProjectSet` projects a universal signed carrier and passes it directly to the same ZeroCrossing planner.
- Universal ABI now exposes generic `PlannedDeltaEffect`, `UnaryDeltaKernel`, and `BinaryDeltaKernel` contracts for the remaining barrier classes.
- Historical production closure remains **14 / 22**. #21/#22 remain integration-in-progress, not closed.

## Hostile falsification

- Γ-equivalent `"A"` / `"a"` rows with support count 2: removing both plans one support crossing while leaving state unchanged until commit.
- a subsequent removal from zero support fails with `InconsistentIncrementalDelta` and leaves the maintained state byte-for-byte equal to its pre-plan clone.
- existing exhaustive/hostile Distinct and set-support tests remain green, including semantic equality rather than Rust equality.
- physical-program classification test covers ZeroCrossing, Annotation, OrderedBoundary, BilinearPullback and BlockerZeroCrossing on real `RelExpr` trees.

## Deliberate non-claims

- Stage 4.2 Annotation/Group is **not started**. The v5 requirement includes the v3 `DenseWindowGroupCount` lowering; Pass96 does not relabel the current BTreeMap fast path as that lowering.
- OrderedBoundary/TopK, BilinearPullback/Join and BlockerZeroCrossing are represented in the compiled physical program but have not yet migrated to universal plan/commit kernels.
- internal `RelationDelta` compatibility materialization still exists after barriers; Stage 5 root-only materialization is therefore not started.

## Final gate

- fmt PASS
- workspace check PASS
- workspace strict Clippy PASS
- full workspace tests PASS
- **689 declared / 0 failed / 8 ignored**
- source delta vs Pass95: exactly three `kernel-query` Rust files
- frozen source fingerprint: `96256411a839e086bc1548ae320c358a02f80593003c279ae62b5b0c5d09ffd7`

## Next

Pass97 should continue Stage 4.2 with Annotation/Group, including the v3 dense-window physical lowering under a fallback/admission policy and differential oracle. Only after Group is complete should it proceed to OrderedBoundary/TopK, then Join and Blocker. Stage 5 remains blocked until every maintained internal barrier consumes/produces the certified ABI.
