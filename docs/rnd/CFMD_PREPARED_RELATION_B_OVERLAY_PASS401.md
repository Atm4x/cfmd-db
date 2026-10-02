# PASS401 R&D — exact prepared relations and current-schema B overlays

## Problem

P398–P400 closed factorized row-local migration and bounded physical materialization, but two coupled gaps remained:

1. a general relational rewrite `q` (`Union`, `Join`, `Difference`, `Distinct`, `Group`, `TopKWithTies`, etc.) could not enter the factorized realization root;
2. current-schema B writes over derived physical leaves must never require an inverse migration back into schema A.

The hostile constraint is one current semantic world. Schema A may remain historical authority or physical input, but cannot reappear as a current query/write branch.

## Selected general-relation law

For a verified migration relation rewrite

```text
q : A[D] -> B.R
```

let

```text
D(q) = exact set of source relations scanned by q.
```

Preparation is performed before semantic cutover:

```text
P_R = columnize(q(eval(rho_A | D(q))))
```

`P_R` contains native target-B column atoms plus the exact source-relation and source-atom dependency closure used to produce them. It is not a second current schema and not a migration-progress bitmap.

Cutover installs only direct B realization rules:

```text
rho_B(R.column_i) = Direct(P_R.column_i)
```

The unprepared compiler remains fail-closed. There is no route to an A-schema current-world query fallback.

### Extensional cutover lemma

Because `SchemaMigrationProgram::verify` prepares the same deterministic `RelExpr` against the source `SemanticContext`, and `prepare_general_relation_factorized` evaluates exactly that query over exactly `D(q)`, the installed native relation satisfies:

```text
evaluate_B(rho_B.R) = q(evaluate_A(rho_A | D(q))).
```

The cutover metadata cost is proportional to target relation schema/column count. The unavoidable data-dependent work is moved into explicit preparation.

### Dependency lemma

Preparation records:

```text
semantic dependencies = D(q)
physical dependencies = union deps(rho_A.relation[d]) for d in D(q)
```

No unrelated source relation is materialized for the query. After cutover, the target relation itself depends only on prepared B atoms; old source atoms remain current-reachable only if another current coordinate still references them.

## Current implementation boundary

The generic preparation executor currently reuses `kernel-query` as the single exact relational semantics and reconstructs only the query's dependency relations into its `FiniteModel` input. This is intentionally not a SQL fallback and does not duplicate query semantics.

It is, however, still row-materializing during preparation. A later executor optimization may compile the same `RelExpr` into a streaming/factorized preparation DAG. That optimization must preserve one algebra and one certificate; it must not introduce a separate "fast query" semantic engine.

## B-write law

For a derived current-B coordinate:

```text
B.c = f(old physical atoms)
```

an already-certified semantic B write to value `v` is realized as:

```text
1. materialize the bounded current-B physical segment containing c;
2. replace c inside that B-native segment;
3. publish a new immutable B-native atom;
4. retarget only the realization overlay to the new atom.
```

No `f^-1` exists or is requested. This works even when `f` is non-injective.

Implemented primitives:

- `install_field_value_overlay`;
- `install_relation_cell_overlay`.

These are physical endpoint installers, not a conflict/rebase engine. Semantic write authority remains in `kernel-change`; migration code does not invent a second write-coordinate calculus.

## Important obstruction for arbitrary global relation results

A general query result does not necessarily expose a stable writable row identity:

- `Distinct` identifies Γ-classes, not source rows;
- `Group` creates derived rows;
- `Union` may merge sources;
- bag multiplicity has no canonical occurrence identity by default.

Therefore the statement "every global derived relation row can be locally written through a lens" is false in general.

The universal semantic direction is a B-native relation endpoint/delta overlay keyed by certified B semantic coordinates. Local physical chunk lowering is valid only when such a coordinate has already been resolved. This is why PASS401 does not add an inverse-query or migration-specific conflict router.

## Performance hostile

Fixture: bag `Union` of two 50k-row source relations, one target column, release build.

Three warm runs:

```text
prepare 100k target rows: 27.471–28.930 ms
cutover root composition:  39.769–43.053 us
source relation deps:      2
source physical atoms:     2
prepared target atoms:     1
```

The relevant asymptotic result is:

```text
preparation = O(data touched by q)
cutover     = O(target schema/layout)
```

No current-world general-query fallback remains on the prepared path.

## Next R&D target

Compile the exact `RelExpr` preparation phase into a factorized/streaming operator DAG so simple global operators avoid temporary row reconstruction while preserving the same `PreparedFactorizedRelation` law. In parallel, define the universal B-native relation delta overlay over `kernel-change`/`kernel-query` semantic coordinates so writes to globally derived relations do not depend on unstable physical row ordinals.
