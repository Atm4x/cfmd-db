# IMPLEMENTATION REPORT — Pass49

## problem

Pass48 correctly eliminated final order restoration, but its transient multiway pruning was still represented as pairwise compatibility masks mirroring syntactic equality edges. That ignored CFMD-specific semantic structure already available from pinned Γ: checked equivalence refinement, transitive quotient equality, redundant equality cycles/cliques and correlations between multiple quotient coordinates carried by one row.

## hypothesis

For an equality predicate graph whose edges carry versioned equivalence laws, define `E1 ⪯ E2` when Γ proves that `E1` refines `E2`. For each target law `E`, every connected component of the graph containing edges whose laws refine `E` is one sound `E`-quotient coordinate. Primitive canonical keys can physically represent that quotient exactly. Multiple quotient coordinates can then propagate support to a finite deletion fixed point while final output remains enumerated in original Bag order and is exact-Γ revalidated.

## implementation

- replaced `JoinPredicateCompatibility` pairwise-mask representation with `SemanticQuotientConstraint` / `SemanticQuotientLeaf`;
- added Γ-dependent refinement-closure component construction through `SemanticRegistry::equivalence_refines`;
- collapses redundant equality cliques/cycles into quotient coordinates;
- removes same-component coarser constraints when a checked finer law implies them;
- caches canonical keys once per `(leaf,column,target equivalence)` including derived coarser coordinates;
- combines multiple same-leaf columns in one quotient coordinate and rejects row-local key disagreement;
- computes N-way common canonical-key domains per quotient coordinate;
- computes monotone cross-coordinate support masks to a finite fixed point;
- keeps original-order leaf/row enumeration and exact original-predicate revalidation;
- replaced syntactic compatibility-build costing with costing over actual derived quotient coordinate specs;
- added execution stats for quotient-constraint count and quotient-pruned rows.

## hostile falsification

Verified:

- `TextExact -> TextAsciiCaseInsensitive` closure derives `{A,B,C}/ASCII-CI` from `A Exact B` + `B ASCII-CI C`;
- the same semantic IDs produce a different physical basis when Γ pins different module contracts;
- same-endpoint coarser duplicate is removed only under checked refinement;
- a six-edge four-coordinate equality clique becomes one quotient coordinate;
- same-row columns in one quotient coordinate must have the same canonical class;
- Exact + ASCII-CI mixed execution equals logical reference semantics;
- pruning propagates through two different quotient coordinates to a fixed point;
- existing 3-way/4-way order-preserving hostile cases remain exact;
- final original Γ predicates remain the authority boundary;
- no new lint suppression, unsafe or TODO-shaped debt was introduced.

## verification/result

Full Rust 1.98.1 fmt/check/debug/release/strict-Clippy/release-build/strict-rustdoc/overflow-release gate: PASS.

Metrics: 356 declared tests, 128 `kernel-plan` tests (127 normal + 1 ignored benchmark), 21 crates, 48,233 Rust LOC, 0 external Cargo sources, 0 `unsafe`.

Diagnostic hostile benchmark remained ~147.4x faster than the deliberately bad original parenthesization on the established three-way fixture; this is not a universal speed claim.

## rejected routes

- did not invent a new logical Join/equality primitive;
- did not claim academic novelty for quotienting/constraint propagation in general;
- did not use Rust `Eq/Hash/Ord` as semantic proof;
- did not remove final exact Γ revalidation;
- did not persist execution-local quotient state as authority;
- did not force WCOJ or another named specialist algorithm when the query hypergraph does not justify it;
- did not relax the current 3–8 leaf search cap merely because factorization is cleaner.

## recommended next step

Compile Γ-QCN basis as checked prepared-plan physical metadata, then investigate exact incremental maintenance through the existing Change/Dq machinery. Only after that should the multi-family advisor decide when to retain/share quotient factors versus semantic/I64 indexes or other layouts.
