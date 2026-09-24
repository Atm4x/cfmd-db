# Pass74 hostile review — R&D Program 9 / Durable Typed Layout Recovery

Status: **ACCEPTED AFTER HOSTILE REVIEW AND INTEGRATED**.

Input: `CFMD_RND_PROGRAM9_CLOSEOUT_PASS72_2026-09-21(1).zip`.
Authoritative integration baseline: Pass73 plus the already-started Pass74 bounded-cyclic prefix-index refinement.

## Package integrity

`SHA256SUMS.txt` verifies every package member. The Program9 patch applies to the current Pass74 tree with `patch -p1 --fuzz=0`; all hunks apply, with only line offsets in `kernel-plan` caused by Pass73/Pass74 planner work.

## Authority review

Program9 persists **logical physical-lowering recipes**, not physical identity. The durability layer contains no `PhysicalRowId`, `LocalEntityId`, `DenseEntityIds`, pointer, or revision-local dense-table serialization. Recovery rebuilds physical relations and I64 indexes from the recovered authoritative Revision and current pinned Γ.

Typed `LiveEntityRef` reconstruction obtains the recovered Revision's fresh dense table, then rebuilds dense local columns from external entity IDs. Therefore the recovered dense table may differ physically while denoting the same logical entities.

Stale or contradictory relation-layout recipes remain non-authoritative advice. Failure to lower a valid-but-stale physical recipe falls back deterministically to `RECOVERY_ROW_STORE`; it does not block logical recovery. Unknown/corrupt recipe format remains fail-closed at metadata decoding.

## Compatibility review

Recipe format is raised to v2 while the v1 decoder remains supported. Program9 composes with Pass71 durable semantic-index/QCN/statistics recipes rather than replacing them.

Recovered I64 index state is reconstructed only after a compatible native relation layout exists. Existing Γ validation on `MaterializedI64IndexState::build` remains the semantic gate; stale equivalence/schema coordinates therefore drop the derived optimization instead of creating authority.

## Decisive hostiles rerun on Pass74

- `physical_artifact_recipe_*` codec/version/manual-pin tests — PASS;
- `durable_typed_layout_and_i64_index_rebuild_and_serve_after_compaction` — PASS;
- `durable_layout_recipe_round_trips_supported_native_representations` — PASS;
- `durable_typed_live_ref_layout_rebinds_fresh_revision_local_dense_ids` — PASS;
- `stale_durable_layout_recipe_falls_back_without_blocking_logical_recovery` — PASS;
- `conflicting_durable_layout_recipes_fall_back_deterministically` — PASS.

Pass74 cyclic-prefix hostiles were rerun after integration and remain PASS.

## Result

Program9 is accepted. It closes the concrete defect that supported typed/columnar layouts and exact I64 indexes were lost across reopen and forced through generic row-store reconstruction. It does **not** close the broader recovery/economics historical item: future layout families, rebuild economics/advisor policy, historical durable-format calculus, streaming checkpoints and machine/filesystem power-loss proof remain separate.
