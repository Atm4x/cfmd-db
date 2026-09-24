# PASS77 Γ-APNF R&D integration review

Input: `CFMD_RND_GAMMA_ANCHOR_PULLBACK_NORMAL_FORM_2026-09-21(1).zip`  
Input SHA-256: `411efa2621d6113f187ff77b2a5c1cc1d8fc7a19a434e2031da32a38c6fb31ed`

Decision: **accepted as semantic foundation, not as a replacement executor**.

The R&D closeout resolves the three mathematical questions left by the previous Γ-observable/measure work in a form compatible with CFMD's authority rules:

1. hyper-determinants are ordinary certified morphisms whose source and target are finite observable products;
2. the semantic deterministic object is the least determinant closure operator, not a minimum FD cover or a particular join/SCC tree;
3. every finite factor admits a lossless anchor-measure + deterministic reconstruction representation, leaving one anchor-pullback residual object.

## Hostile review conclusions

### Accepted

- n-ary -> m-ary determinants require no unary-only production API;
- closure is an inflationary monotone fixed point on a finite observable set and can be executed with an incidence worklist;
- reverse-delete is suitable only as a deterministic inclusion-minimal generator, never as a minimum-cardinality claim;
- logically redundant direct/composed morphisms must remain legal physical accelerators;
- anchor projection must be injective only on distinct support tuples; Bag multiplicity belongs to the anchor measure, not to row identity;
- the residual executor may later consume the same APNF object, while GYO/QCN remain migration-time physical strategies rather than semantic authority.

### Production corrections made during integration

The standalone prototype used abstract integer coordinates/atoms. Production integration does not copy that surface:

- coordinates are `RevisionObservableId` and values are `EqClassId`;
- every factor, morphism, determinant theory and APNF object is bound to one `SemanticRevision` and one observable-catalog realization;
- external callers cannot manufacture a `CertifiedSemanticMorphism` from arbitrary observation samples. The raw observation constructor is crate-internal; the public finite-factor path derives a morphism only after checking the complete factor support supplied to that factor object;
- hyper-determinant source/target tuple order is preserved explicitly when materializing the finite mapping;
- stable generator/basis selection is query/factor-order deterministic and is reconstructible state, not durable semantic authority.

## Integrated production substrate

`kernel-semantics::anchor_pullback` now contains:

- `RevisionFiniteMeasure`;
- `AnchorMeasureState` + weighted lossless reconstruction;
- safe finite-factor hyper-determinant derivation;
- `DeterminantTheory` with least closure and incidence/worklist execution;
- value-level exact morphism saturation with undefined-source/conflict rejection;
- stable inclusion-minimal determinant generators;
- `AnchorPullbackNormalForm` as a query-local semantic core;
- determinant-backed branch-free certificates.

The existing Pass77 observable layer supplies revision-local observable/class identity, product observables and n-ary -> m-ary `CertifiedSemanticMorphism`.

## Deliberate non-integration

Pass77 does **not**:

- lower arbitrary `RelExpr` to APNF;
- replace QCN/GYO/current bounded cyclic execution;
- claim a minimum candidate key/FD cover;
- materialize a global residual fiber profile;
- implement the general residual anchor-pullback solver;
- add durability or incremental maintenance for APNF derivatives.

Those remain engineering work on top of the now-stable semantic substrate.
