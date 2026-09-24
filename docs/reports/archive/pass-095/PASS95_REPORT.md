# PASS95 REPORT

**Status:** FINAL / V5 DELTA KERNEL STAGES 2–3 INTEGRATED

## Baseline and wall-clock boundary

Baseline: frozen Pass94 (`cfmd_workspace_pass94_consensus_vote_delta_abi_stage1.zip`). Source work started at **2026-09-23 00:32:12 UTC**. Production source was explicitly frozen at **00:47:08 UTC**, well before the 20-minute source boundary. Stage 4 was deliberately not started. After freeze only reports, manifest and packaging are changed.

## Result

- V5 #21/#22 integration **Stage 2 complete**: proof-producing `ValidatedTransitionFrame` retains the already-computed source mutation plan and certified effect; Scan commit consumes the frame instead of replanning.
- V5 integration **Stage 3 complete**: `RelDifferentialProgram` now derives `CompiledDeltaProgram` with maximal `LinearIslandNormalForm` chains and a Γ-aware normal-form executor.
- Historical production closure remains **14 / 22**. #21/#22 are integration-in-progress, not closed. #16 is unchanged from Pass94.
- Stage 4 barrier migration has **zero production changes** in this pass.

## Hostile falsification

- same base relation appears twice in a self-join: separate compiled edge identities retain separate prepared leaf patches and maintained output equals full recomputation;
- Filter -> ProjectBag -> Filter chain compiles to one island, both predicates map back to source column coordinates, and fused execution equals existing maintained propagation;
- Set/Distinct/stateful classes are excluded from the linear island compiler.

## Final gate

- fmt PASS
- workspace check PASS
- workspace strict Clippy PASS
- full workspace tests PASS
- **687 declared / 0 failed / 8 ignored**
- source delta vs Pass94: exactly three `kernel-query` Rust files
- frozen source fingerprint: `8a574d7e582b049a33f04de1359296ad1fee8daffef45a7afb9567e18e753ddc`

## Next

Pass96 should begin V5 **Stage 4** at the first barrier class, ZeroCrossing (`Distinct` / `ProjectSet`), and proceed class-by-class only after differential parity. If time permits after complete barrier migration, continue Stage 5 root-only `RelationDelta` materialization and Stage 6 production performance gates.
