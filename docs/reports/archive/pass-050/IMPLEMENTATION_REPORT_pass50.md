# IMPLEMENTATION REPORT — Pass50

## problem

Pass49 Γ-QCN rebuilt its checked semantic quotient basis on every prepared execution and recanonicalized every quotient endpoint from row payloads even though both layers have stronger reuse boundaries in CFMD: the basis is stable for pinned `(query, Γ)`, while primitive canonical endpoint keys are reconstructible revision-derived state that can be maintained by stable row handle across exact relation deltas.

## hypotheses

1. Quotient-basis compilation belongs in `PreparedPlan`, not in semantic authority and not in durable revision state.
2. Primitive quotient factors can reuse the verified `MaterializedSemanticIndexState` representation without becoming ordinary Join access indexes.
3. Keeping quotient factors in a dedicated physical family prevents their existence from silently changing `JoinAccessDecision` or Pass43 advisor ownership.
4. Existing prepared relation-delta machinery can validate and atomically maintain the new factor family together with every other relation-derived family.
5. Full support-mask/fixed-point incrementalization should remain OPEN until derived cleanly from the change calculus; basis/factor reuse alone must not be overstated.

## implementation

- added private `PreparedSemanticQuotientProgram` compiled during `prepare_baseline` / `prepare_with_catalog` from the physical multiway shape and pinned Γ refinement laws;
- prepared native execution reuses the compiled quotient specs; dynamic Plan execution keeps exact dynamic compilation fallback;
- added `semantic_quotient_factors` as a distinct `PhysicalStore` family keyed by `SemanticIndexBinding` but intentionally excluded from ordinary semantic-index access-path lookup/advisor ownership;
- reused `MaterializedSemanticIndexState` as the primitive Γ-canonical stable-handle factor representation;
- added `PreparedPlan::materialize_semantic_quotient_factors`, which prepares all missing/stale factors before one publication epoch and is idempotent when factors are already compatible;
- refactored relation-derived validation/application so specialized I64 indexes, generic semantic indexes, quotient factors and semantic statistics all participate in the same prevalidated physical delta transition;
- relation reinstall invalidates quotient factors; Γ incompatibility requires rebuild rather than reinterpretation;
- quotient key-cache construction preferentially consumes maintained `PhysicalRowId -> CanonicalEqKey` factor evidence and falls back to exact row canonicalization when unavailable;
- added execution evidence counters for prepared quotient-program reuse and maintained quotient-key hits.

## hostile falsification

- quotient factors are deliberately invisible through the ordinary semantic-index family;
- repeated factor materialization is a no-op rather than a second publication;
- prepared Γ-QCN execution equals fresh logical reference evaluation;
- a 140-row hostile fixture receives all 140 endpoint canonical keys from maintained factors;
- after exact relation delta, the factor is updated atomically and the next prepared execution again receives all 140 keys from factor state while matching logical reference;
- no new `#[allow]`; strict Clippy remains clean;
- no claim is made that common domains/support masks/support fixed point are maintained incrementally.

## verification/result

Final Rust 1.98.1 fmt/check/debug-workspace/strict-Clippy/release-workspace/release-build/strict-rustdoc/overflow-release gate: PASS. External compilation timeouts in combined/warm-up invocations were not counted as results; each unfinished stage was rerun separately to completion.

Metrics: 357 declared tests, 129 `kernel-plan` tests (128 normal + 1 ignored diagnostic benchmark), 21 crates, 48,758 Rust LOC, 19 pre-existing `#[allow]` sites and no new suppression, 0 external Cargo sources, 0 `unsafe`.

## rejected routes

- no generic semantic-index installation merely to cache Γ-QCN factors: that would alter Join access economics and ownership;
- no durable semantic authority for quotient factors;
- no hidden claim of full incremental Γ-QCN maintenance;
- no bespoke support-state invalidation graph before proving it composes cleanly with existing Change/Dq semantics;
- no removal of exact final Γ predicate revalidation.

## recommended next step

Attempt a small, hostile incremental quotient-domain/support-state slice driven by exact relation deltas and stable handles. Require equivalence to fresh Γ-QCN recomputation after every transition. If clean compositional maintenance cannot be obtained without operator-specific authority/invalidation duplication, keep support state execution-local and move instead to lifecycle/memory/adaptive representation work.

## remaining risks

The historical 22-item OPEN ledger remains intact. Immediate Γ-QCN risks are execution-local common-domain/support reconstruction, explicit rather than advised quotient-factor lifecycle, primitive-only canonical factors, bounded search and lack of byte-level factor budgeting/sharing policy.
