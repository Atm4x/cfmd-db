# Pass79 R&D review — Γ Grounded Closure Calculus

Input reviewed: `CFMD_RND_GAMMA_GROUNDED_CLOSURE_CALCULUS_2026-09-21(1).zip`.

## Accepted claims

The central finite-support claim survives mainline hostile integration: APNF determinant saturation and ordinary reachability are the same least grounded closure problem after lowering to finite hyperrules. Groundless cycles remain false; genuine n-ary bodies require all premises; rank + selected-witness certificates provide groundedness rather than mere closure.

The witness-cone deletion law is also integrated in the generic substrate. Random mixed seed/rule updates are checked against full rebuild, including the hostile case where deleting a non-selected redundant rule performs zero invalidation work and the case where removal of the only grounding seed kills a self-supported cycle.

## Production integration

Pass79 introduces `kernel-grounded-closure`, intentionally below schema/Γ/physical layers. It owns only dense finite atom/rule IDs, incidence structures, least-closure evaluation, certificate checking and local update mechanics. It is not semantic authority.

`kernel-semantics::DeterminantTheory::closure` now lowers revision-local observable coordinates to this substrate. Certified semantic morphisms remain the authority from which hyperrules are compiled.

`kernel-fixpoint::solve` now lowers reachability to the unary fragment of the same substrate and maps the grounded witness certificate back to the existing `ReachabilityCertificate`. The independent existing checker remains unchanged.

`kernel-lifecycle::MaintainedDenseLifecycle` is deliberately not replaced. Its dense specialized algorithm remains a physical lowering. An exhaustive three-node parity oracle now proves its reference liveness semantics agrees with unary Γ-GCC.

## Rejected / deferred claims

Pass79 does not expose unrestricted recursive Bag/Natural multiplicity. No convergence claim is made outside finite/idempotent support carriers.

Pass79 does not claim that a single physical implementation should replace specialized dense lifecycle traversal. Semantic unification and physical unification are separate questions.

Pass79 does not yet lower recursive `RelExpr` rule bodies into grounded atoms/hyperrules. That requires the R&D frontier around Γ observables/APNF/SAMF and residual recursive support to settle.
