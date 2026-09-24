# Pass70 hostile review — R&D Program6 Compiled Γ Runtime

Date: 2026-09-21
R&D input: `CFMD_RND_PROGRAM6_CLOSEOUT_PASS68_2026-09-21.zip`
Authoritative integration base: verified Pass69
Decision: **ACCEPTED AFTER SEMANTIC REBASE + HOSTILE HARDENING**

## What was accepted

Program6's central design is sound: `CompiledEquivalence` is a reconstructible executable cache of pinned Γ, not a new equality authority. Primitive leaves retain resolved certified equivalence contracts; structural nodes retain direct child-node indices. Product/Option/Sum/Seq/Set/Bag/Map and guarded Mu/Var are compiled once and canonicalized without repeatedly walking schema/module lookup on every row.

The Pass68-relative patch was not applied fuzzily. Clean hunks were accepted, while Pass69-overlapping state/binding code was rebased manually.

## Pass69 rebase work

Pass69 introduced structural semantic indexes and a versioned key-binding contract after Program6's Pass68 baseline. Pass70 therefore extends compiled Γ to that new consumer as well:

- primitive semantic-index key parts preserve the direct `ResolvedPrimitiveEquivalence` fast path;
- structural semantic-index key parts retain `CompiledEquivalence`;
- build/probe/delta maintenance use the compiled program rather than returning to registry structural interpretation;
- Γ-QCN quotient factors retain compiled programs and validate them together with Pass69 key bindings;
- algebraic structural typed filters compile once per execution and walk native structural storage through the compiled node program.

## Hostile finding fixed during integration

Pass69's original `SemanticIndexBinding` trusted nominal `SchemaRevisionId` plus primitive module digests. Two different structural definition graphs could therefore reuse the same nominal schema/environment revisions and the same primitive dependency set while assigning those primitive laws to different structural positions.

That was an exactness hole independent of Program6, and a stale compiled cache would make it more visible.

Pass70 fixes it by binding long-lived canonical-key caches to the exact structural-definition closure. Compatibility now has an explicit `RebuildStructuralDefinitions` outcome. A regression swaps exact/ASCII-CI leaf laws across Product fields while keeping the same nominal revision IDs and the same primitive dependency set; reuse is rejected.

The retained-memory estimator also charges the structural-definition binding payload instead of making that certificate invisible to advisor budgets.

## Falsification

- compiled canonical keys match the existing registry oracle for primitive and composed structural laws;
- guarded recursive Mu/Var remains accepted, free/unguarded recursion remains rejected;
- algebraic structural canonicalization matches the registry oracle;
- structural semantic-index Filter remains exact and delta-maintained while using compiled structural parts;
- Γ-QCN factor delta maintenance retains exact results with compiled keys;
- same-nominal-revision structural-definition drift is rejected;
- Pass69 canonical-key v1 golden/hostile codec tests remain green;
- Pass68 durability and Pass67 advisor/COW suites remain green.

## Non-claims

Program6 does not make compiled Γ semantic authority and does not serialize compiled programs durably. Revision-level sharing of compiled programs across every consumer, compiled ordering/tokenizer programs and vectorized predicate/mask bytecode remain future optimization fronts.
