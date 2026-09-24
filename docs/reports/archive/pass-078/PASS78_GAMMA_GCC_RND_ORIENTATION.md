# Pass78 Γ Grounded Closure Calculus R&D orientation

Input reviewed after source freeze: `CFMD_RND_GAMMA_GROUNDED_CLOSURE_CALCULUS_2026-09-21(1).zip`.

## Decision

Architecturally compatible and promising, but **not integrated in Pass78**. It changes the semantic closure substrate rather than the SAMF physical partition, so it can be developed independently in the next pass.

## Core result accepted for further hostile review

The R&D identifies one finite grounded least-closure calculus over n-ary hyperrules behind:

- APNF determinant saturation;
- ordinary reachability / lifecycle liveness as the unary fragment;
- future positive recursive support over a certified finite-height/idempotent carrier.

A groundless cycle must remain false. Rule incidence worklists give finite structural work, and proof/witness information supports local deletion re-fixpoint without blindly invalidating every descendant.

The bundle reports randomized parity with Pass77 `kernel-fixpoint` and `MaintainedDenseLifecycle`, genuine hyperrule/cycle hostiles, and mixed update tests.

## Integration rule

Follow the bundle's conservative migration order:

1. introduce one generic explicit finite-hyperrule grounded-closure substrate + checker/certificate;
2. port `DeterminantTheory::closure` first while retaining current code as oracle;
3. lower `kernel-fixpoint` to unary rules and compare with existing BFS;
4. only after long randomized parity, trial lifecycle lowering against `MaintainedDenseLifecycle`;
5. introduce recursive-query `MuSupport` only for certified finite-height/idempotent support semantics.

Do **not** infer automatic convergence for unrestricted recursive Bag/Natural multiplicity.
