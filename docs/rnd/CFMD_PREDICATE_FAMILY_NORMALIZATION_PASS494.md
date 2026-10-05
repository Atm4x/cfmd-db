# PASS494 R&D — Predicate-family normalization boundary

## Question

Can PASS493 equality/order family fusion be extended into one universal exact family lowering for richer conjunction/range predicates, without adding a second predicate/query engine beside `RelExpr` / `RelObservationForest`?

## Result

Only in one dimension.

For a conjunction of constant order predicates that all observe the same canonical semantic coordinate `(parent, column, ordering)`, every predicate is a lower or upper cut in the quotient order produced by the pinned `CompiledOrdering`. Their conjunction therefore has one exact interval normal form:

```text
lower = strongest of { x > a, x >= a }
upper = strongest of { x < b, x <= b }
interval = lower ∩ upper
```

Equal-bound inclusivity composes by conjunction. Contradictory bounds produce the empty interval. This is an exact algebraic normalization over canonical order keys, not a host-language optimization.

PASS494 adds executable coverage showing a four-predicate chain

```text
x >= 2 AND x > 1 AND x <= 5 AND x < 6
```

is exactly `[2,5]` in canonical ordering space and agrees with ordinary `RelExpr` evaluation.

## Hostile boundary — more than one independent coordinate

For predicates on two independent ordered columns, the normal form is no longer one interval. It is a product region (orthotope):

```text
I_x × I_y × ...
```

A single existing scalar family key is not a sufficient statistic for membership.

PASS494 includes an executable four-quadrant counterexample. Rows with the same canonical `x` key but different `y` values route to different conjunction roots, and rows with the same canonical `y` key but different `x` values also route differently. Therefore neither `(column=x, ordering)` nor `(column=y, ordering)` can replace the full conjunction semantics.

A tuple/multidimensional key could of course represent the row, but efficient fanout then becomes an orthogonal range-reporting / multidimensional decision-index problem. That is a new indexing algebra with different construction/update/memory tradeoffs, not a continuation of PASS493's one-dimensional cut theorem.

## Architecture decision

Do **not** add conjunction-specific routers to `RelObservationForest` now.

The universal law established by PASS493/PASS494 is:

```text
one semantic scalar quotient coordinate
    -> exact equality dispatch or exact ordered interval/cuts

multiple independent semantic coordinates
    -> product-region semantics; no single scalar family lowering
```

The database may eventually gain a general multidimensional semantic index if product workloads justify it, but it must be designed as a first-class Γ-aware index algebra and proved independently. It must not appear as an ad-hoc recovery/predicate cache.

This closes the current parameter-family specialization line. The next productization work should return to breadth: ensure ordinary object/reference/relationship/traversal reads that lower to relational semantics actually capture and preserve the same exact causal authority, then resume the deferred Context/Auth/Semantic Rules lines.

## Rejected continuations

- nested-filter pattern matching with one-off special routers;
- flattening multi-column conjunctions into a fake single order key;
- host tuple/hash dispatch that ignores pinned Γ equivalence/ordering semantics;
- materializing all conjunction truth vectors per row;
- a second predicate evaluator beside `RelExpr` / Γ-DTC;
- claiming sub-`output` work when one changed row genuinely belongs to many roots.

## Executable obligations

- `p494_one_axis_order_conjunction_has_exact_interval_normal_form`
- `p494_multiaxis_conjunction_has_no_single_scalar_family_key`

The first is the positive theorem for one-dimensional ranges. The second is the hostile obstruction that stops further scalar-family specialization.
