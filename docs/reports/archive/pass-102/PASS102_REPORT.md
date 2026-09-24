# PASS102 REPORT

**Status:** FINAL / V5 STAGE 5 COMPLETE / ROOT-ONLY COMPATIBILITY MATERIALIZATION

## Baseline

Frozen Pass101 (`cfmd_workspace_pass101_blocker_stage4_complete.zip`), SHA-256 `4bedc56f347483efaf1dab59b2052820a2ba5a36246b8e078c932132a25f4dec`.

Pass101 was restored from the persisted checkpoint after the previous sandbox loss and verified by ZIP integrity before any edits. Rust 1.98.1 was reinstalled from the supplied distribution, smoke-tested independently (`pass102-rust-smoke:31`), and the source tar/staging were deleted before CFMD work.

## Result

- **V5 Stage 5 — COMPLETE.**
- Internal recursive maintained edges no longer construct or return `RelationDelta`.
- Internal carrier: `AdaptiveDelta<Row,4>` / `DeltaView<Row>`.
- Ordinary public leaf ingress remains `RelationDelta`; storage-resolved ingress remains `StorageResolvedRelationDelta`.
- Successful maintained transitions materialize compatibility `RelationDelta` exactly once at the root/public result boundary.
- Failed transition used by the hostile gate materializes zero compatibility results and leaves state unchanged.
- All Stage 4 barrier kernels forward their carrier-native `planned.effect` directly.
- Production source delta versus Pass101 is exactly one Rust file: `crates/kernel-query/src/lib.rs`.

## Exact architectural changes

1. `MaterializedRelPlanState::apply_relation_deltas_inner` returns `MaintainedDelta` rather than `RelationDelta`.
2. `propagate_resolved_deltas_inner` uses the same carrier.
3. Scan commits its already-validated public leaf mutation and moves/clones only the signed effect into the carrier.
4. Filter / FilterColumns preserve signed weights without compatibility conversion.
5. ProjectBag retains Γ-aware inserted/removed cancellation after projection, then emits the carrier.
6. ProjectSet / Distinct forward `MaterializedSetSupportState::plan_delta_view(...).effect` directly.
7. PromoteToBag is a carrier pass-through.
8. Join, Difference/AntiJoin, Group and TopK call their Stage-4 planners directly and forward `planned.effect` after committing the checked patch.
9. Root ordinary and storage-resolved entrypoints call `materialize_delta_view` once after successful recursive propagation.

## Hostile materialization gate

The existing deep recursive maintained-plan test covers Filter, Project, Join, Group and TopK plus both ingress modes. Test-thread instrumentation around `materialize_delta_view` proves:

- ordinary successful transition: `1`;
- storage-resolved successful transition: `1`;
- invalid transition: `0`.

The test simultaneously compares maintained output/state with full recompute, so the count cannot pass by skipping semantic work.

## Verification

- formatting — PASS;
- workspace check — PASS;
- strict workspace Clippy (`-D warnings`) — PASS;
- full workspace all-target tests — PASS;
- **700 declared / 692 passed / 0 failed / 8 ignored**.

Production fingerprint: `dffeef1669763a37c9f48f7a753acc25d048513ea2005aefd2b17be1e5e64964`.

## Historical status

Historical production closure remains **14 / 22**. #21/#22 are intentionally not closed yet.

## Deliberate non-claims

- Stage 6 is not complete.
- No whole-chain speed/allocation win is claimed merely from removing `RelationDelta` objects.
- ProjectBag semantic normalization still uses temporary vectors.
- Existing generic fallback allocations are not asserted eliminated.
- EXECGRAPH V3 `PreparedRelGraph / CompiledTransitionProgram` is still not merged; it becomes eligible for rebase only after this Stage-5 boundary and after Stage-6 evidence establishes the current carrier baseline.

## Next

Pass103 / Stage 6: corrected whole-chain differential benchmarks and allocation evidence across representative linear + blocker + join + group + top-k chains, compare against Pass101/legacy compatibility behavior where reproducible, attack fallback allocation hotspots, and only then decide whether historical #21/#22 satisfy PROD CLOSED.
