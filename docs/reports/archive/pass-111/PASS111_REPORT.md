# PASS111 — Engineering closeout: typed direct-flat build

Freeze: 2026-09-23T14:52:34,957802329+00:00
Production fingerprint: ab29f02d899a7f563a870179b03a190b01829520ff2136d30f1cd55b21657ca9

## Result
Pass111 closes the remaining maintained-plan engineering construction debt without changing relational semantics, Stage-6 closure, or the 16/22 historical production count.

### Engineering item 1 — full compile-once typed metadata: CLOSED
- `PreparedRelGraph` now owns typed postorder metadata (`NodeId -> RelType`) for production compilation.
- One validating `RelExpr::typecheck()` traversal establishes semantic correctness; downstream physical/differential/maintained compilers consume the typed graph instead of recursively re-typechecking subtrees.
- Recursive `typecheck()` calls were removed from the maintained builder, `linear_island` compilation, barrier collection, and `RelDifferentialNode` compilation.
- The former `build_subtree()` path no longer exists.
- This removes the old deep-tree repeated-subtree / O(n^2)-shaped type traversal debt.

### Engineering item 2 — recursive construction representation: CLOSED
- Maintained state is built directly into the authoritative flat NodeId arena in postorder.
- `flatten_runtime_arena()` and recursive construction builders were removed.
- Release builds compile out `MaintainedRelPlanNode` and the `node` field entirely.
- Debug builds may rehydrate a recursive tree *after* flat construction solely as the independent differential oracle. It is not construction state and is not release runtime ownership.
- Release structural gate `release_maintained_plan_is_direct_flat_arena` passes.

### Engineering item 3 — attach_storage_rows whole-state candidate clone: CLOSED in Pass110
Revalidated in Pass111. `attach_storage_rows()` validates first and performs targeted arena COW mutation; it no longer begins with `candidate = self.clone()`.

### Engineering item 4 — revision candidate shallow clone: INTENTIONAL / NOT A DEBT
`candidate_from_storage_resolved_deltas_for_revision()` still begins from `self.clone()`. With the flat Arc/COW arena this is a shallow detached-candidate snapshot and is required by publication semantics. No deep runtime-state copy occurs until touched slots COW.

### Engineering item 5 — standalone I64 Group vs hand microbaseline: NON-BLOCKING / RETAINED EVIDENCE
The ~4–5x hand-written micro-ceiling remains documented. Stage 6 and Historic #21 are already correctly CLOSED because corrected whole-chain, allocation, oracle, atomicity, and stale-revision gates pass and the V5 closure criterion does not require equality with a fused hand baseline.

## Gates
- `cargo fmt --all -- --check`: PASS
- `cargo check --workspace --all-targets`: PASS
- `cargo clippy --workspace --all-targets -- -D warnings`: PASS
- release strict Clippy: PASS
- kernel-query: 104 passed / 0 failed / 1 ignored
- full workspace all-targets: PASS (same 711 declared structure as Pass110: 703 passed / 8 ignored)
- release direct-flat structural assertion: PASS

## Next frontier
Historical status remains 16/22 PROD CLOSED. Resume #18 formal immutable-generation publication/fsync/GC proof, then #16 replication/consensus runtime.
