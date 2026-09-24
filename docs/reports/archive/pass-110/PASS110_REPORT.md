# PASS110 REPORT — problem revalidation + #18 publication model start

## Scope

Revalidate the five residuals listed against the Pass109 Flat NodeId Arena checkpoint, close those that can be closed without architectural churn, then resume the historical frontier in the planned order #18 -> #16.

## Revalidation of the five listed residuals

1. **Full compile-once typed metadata: STILL ACTIVE.** `MaterializedRelPlanState::build_subtree()` still calls recursive `RelExpr::typecheck()` per subtree. This is a build-time traversal debt, not a runtime-state correctness defect. Correct closure should unify typed postorder metadata with `PreparedRelGraph`; no thread-local/cache workaround was introduced.
2. **Recursive construction/debug representation: STILL ACTIVE as build/debug debt.** Release runtime owns only the Flat NodeId arena, but build still constructs `MaintainedRelPlanNode` and then flattens it; debug retains the tree as the differential oracle. This should be closed together with item 1 by a direct typed flat builder.
3. **`attach_storage_rows()` candidate clone: CLOSED Pass110.** The method now validates every matching Scan binding before mutation, constructs handles once, then COW-commits only the arena slots. Failure leaves epoch/state/Arc identity unchanged. A hostile test verifies failure atomicity and COW isolation.
4. **Revision candidate shallow clone: INTENTIONAL / NOT A DEFECT.** The clone is the detached publication candidate contract. With the flat Arc arena this is shallow COW and preserves source-state authority until publication.
5. **Standalone I64 Group ~4–5x hand microbaseline: TRUE BUT NON-BLOCKING.** It remains a measured optimization opportunity, not a reopened #21. Pass108 closed #21 under the authoritative V5 whole-chain/allocation/correctness criteria.

Items 1+2 are now one explicit engineering task: **typed direct-flat build**. They are not split into two pseudo-fixes.

## #18 progress

Added `crates/kernel-durability/src/store/publication_model.rs`, a finite immutable-generation publication/GC model under the store test module.

The model explicitly represents:
- synced generation prerequisites;
- prerequisite directory sync;
- pending-manifest sync;
- manifest rename with pre-dir-fsync uncertainty;
- manifest directory sync;
- obsolete-generation GC removals with arbitrary persistence subsets before final directory sync;
- recovery authority as the highest durable final manifest.

Model invariants/checks now cover:
- pending manifest never being authority;
- unique selected authority;
- selected authority retaining durable generation prerequisites;
- rename uncertainty permitting old/new recovery only after prerequisite closure;
- GC never targeting the current authority;
- arbitrary partial persistence of obsolete removals not making current authority unrecoverable;
- explicit one-to-one mapping of all ten production `StoreFaultPoint` boundaries into formal model boundaries.

Production refinement evidence was rerun:
- subprocess-kill checkpoint/manifest matrix: PASS;
- subprocess-kill compaction matrix: PASS.

No supported theorem prover/model checker (`lean`, `z3`, `cvc5`, `tlc`, `dafny`) is installed in the environment. Therefore #18 remains OPEN/PARTIAL: Rust exhaustive model checking is evidence, not the required final external mechanization.

## Gates

- `cargo fmt --all -- --check`: PASS
- `cargo check --workspace --all-targets`: PASS
- strict workspace Clippy: PASS
- targeted attach-storage atomicity: PASS
- #18 publication model: 4/4 PASS
- real subprocess publication crash matrix: PASS
- real subprocess compaction crash matrix: PASS
- full workspace: **703 passed / 0 failed / 8 ignored = 711 declared**

## Historical status

Still **16/22 PROD CLOSED**.

- #18: OPEN -> **ACTIVE / FORMAL MODEL PARTIAL**
- #16: remains ADVANCED / PROD PARTIAL; no consensus safety claim changed in this pass.

## Next

1. Finish engineering cleanup items 1+2 as one typed direct-flat compile artifact, without disturbing the historical count.
2. Continue #18: export the finite transition system into a mechanically checkable theorem/model-checker artifact once a prover is available; strengthen production refinement to cover streaming checkpoint publication as the same protocol.
3. Then begin #16A: first-class durable term/election/locking authority on the existing vote-once journal.
