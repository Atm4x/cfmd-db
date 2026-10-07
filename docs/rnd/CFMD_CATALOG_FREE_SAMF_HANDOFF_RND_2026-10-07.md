# CFMD CATALOG-FREE SAMF / PRODUCTION HANDOFF R&D — 2026-10-07

## Status

R&D convergence pass. This pass does **not** switch production routing.

The target architecture is now narrow enough for production integration: one revision/store-scoped atomic semantic-class authority, encoded relation columns containing those atomic class IDs, and catalog-free derived fiber/projection structures over tuples of those IDs.

The previous retention-profile / advisor-synthesis line is not the target architecture.

## Starting point

Previous R&D established:

- `RevisionSemanticClassCatalog` owns atomic `(equivalence, canonical key) -> EqClassId` identity and canonical payload;
- relation semantic lanes retain `RowId -> EqClassId` and `EqClassId -> rows` under that shared authority;
- the same lanes exactly drive non-I64 `JoinEq`, `Group`, `Distinct`, `FilterEqConst`, and incremental delta maintenance;
- external-class SAMF can be built from those lanes without re-canonicalizing relation rows;
- current SAMF can wrap global atomic classes as local `ObservableClassSignature::SemanticClass`, proving atomic canonical payload need not be owned by SAMF.

The unresolved question was whether the remaining SAMF-local observable/product catalog was semantically necessary.

## Result: product atoms are not semantic classes

Added `kernel_semantics::support_atom::SemanticSupportFabric<RowId>`.

Its input signature is directly:

```text
[global atomic EqClassId; arity]
```

from the shared revision/store semantic class authority.

It has no dependency on:

- `RevisionObservableCatalog`;
- `RevisionObservableId`;
- a product observable;
- product `EqClassId`s;
- `CertifiedSemanticMorphism`.

A joint atom receives only an internal `SemanticSupportAtomId(u64)`. This ID is a compact local routing token scoped to one fabric. It is not semantic authority, is not durable, and is not compared across independently evolved snapshots.

The exact product identity is the atomic-class signature itself.

## Exact retained law

For a fixed encoded binding signature

```text
kappa_B(row) = (EqClassId_0, ..., EqClassId_n)
```

`SemanticSupportFabric` retains only:

```text
signature -> local atom
local atom -> { signature, rows }
row -> local atom
(slot, atomic class) -> { local atoms }
```

Therefore:

- joint fiber = rows of the atom selected by the full signature;
- projected fiber = union of rows of atoms incident to one coordinate class;
- projected count = sum of those atom masses;
- distinct projected classes = keys of that slot's inverse incidence;
- row semantic key = row -> atom -> global atomic signature -> shared class catalog canonical payload.

No second equality-class namespace is required.

## Executable proof

`semantic_support_fabric_factors_global_atomic_classes_without_observable_catalog` covers:

- multiple rows sharing one exact joint atom;
- distinct joint atoms sharing coordinate classes;
- row -> signature;
- exact joint fiber;
- projected fiber;
- projected count;
- distinct coordinate classes;
- atom retirement after the last row leaves a joint signature.

The kernel-plan external-SAMF R&D test now additionally builds a catalog-free fabric directly from the same revision semantic encoded columns and proves equality with current external-class SAMF for:

- row count;
- atom count;
- projection incidence cardinality;
- projected class cardinality;
- joint probe result;
- slot counts;
- per-row canonical tuple reconstruction through `RevisionSemanticClassCatalog`;
- durable canonical-key tuples.

The catalog-free durable tuple bytes are byte-for-byte equal to the existing ObservableAtom durable core input.

## Source hostile inventory

Outside `MaterializedObservableAtomState`, production code does not consume its product observable or product projection. The public getters are exercised by tests in `test_parts/segment_03.rs`.

Within `MaterializedObservableAtomState`, the remaining private observable catalog is used for:

1. revision compatibility;
2. canonical reconstruction;
3. arity via `observables.len()`;
4. value -> local coordinate class lookup;
5. delta validation/build;
6. product-class interning;
7. maintaining the certified product projection.

Under the converged architecture these map directly to:

1. shared semantic catalog revision;
2. shared semantic catalog `class_key`;
3. binding key-part count;
4. shared semantic catalog lookup + encoded lane;
5. encoded-lane delta maintenance;
6. unnecessary: fabric interns local product atoms itself;
7. unnecessary for the materialized SAMF state; projection incidence is already in the fabric.

This means the remaining dependency is implementation ownership, not a missing semantic theorem.

## Why this is not another profile

There is still one semantic equality law:

```text
Value
  -> pinned equivalence
  -> canonical semantic class
  -> revision/store-scoped atomic EqClassId
```

`SemanticSupportFabric` is a reconstructible physical index over tuples of those IDs. It does not introduce another representation of equality.

Statistics, quotient/grouping, join/filter and SAMF fibers are all derived operations over the same atomic IDs.

## Production handoff decision

**GO for a production integration agent after the verification gates in this report are green.**

The handoff is deliberately narrow. The production agent should integrate the semantic-class substrate and catalog-free SAMF, not the older retention-plan compiler experiments.

### Integrate

1. Promote `RevisionSemanticClassCatalog` and revision semantic encoded columns out of `cfg(test)` into the real `PhysicalStore` snapshot substrate.
2. Keep atomic semantic IDs physical/reconstructible; do not expose or persist `EqClassId` as semantic authority.
3. Replace SAMF's atomic `RevisionObservableCatalog` ownership with shared encoded lanes.
4. Replace product `EqClassId` authority in `SupportAtomFabric` with local fabric atom identity as proven by `SemanticSupportFabric`.
5. Reconstruct canonical keys only through the shared semantic class catalog when an API/durable boundary needs them.
6. Drive Join/Group/Distinct/equality-filter from the same encoded lanes.
7. Maintain catalog references and encoded lanes atomically with relation delta; retain-before-insert and fabric-remove-before-final-release ordering must preserve class liveness inside the candidate snapshot.
8. Preserve the existing durable format initially: durable ObservableAtom canonical-key tuples remain reconstructible and were proven equal in this R&D.

### Do not integrate

- retention-plan/profile synthesis as production authority;
- advisor-selected semantic meaning;
- SharedCanonicalKey as another semantic family;
- full-relation Cartesian product atoms;
- durable/public `EqClassId` identity;
- a second canonical-key owner inside SAMF.

The earlier R&D remains useful as hostile/performance evidence, not as the target architecture.

## Remaining production work, not R&D ambiguity

The remaining work is implementation/gating rather than an unresolved mathematical model:

- migrate existing `MaterializedObservableAtomState` to catalog-free fabric;
- remove obsolete product-observable/projection API and tests or restate them against fabric signatures;
- update uniqueness violation keys away from product `EqClassId` if that API remains necessary;
- update memory accounting for shared atomic catalog + encoded lanes + local fabric;
- run full kernel-plan/whole-repository regression and release benchmarks;
- decide materialization/admission policy for encoded lanes separately from semantic correctness.

A performance/admission policy may decide whether a reconstructible index is retained, but it must not select between different semantic laws.

## Production kill criteria

Abort/revisit the integration if any of these become necessary:

- a second canonical-key namespace inside SAMF;
- cross-snapshot meaning assigned to local fabric atom IDs;
- a new planner semantic family solely for encoded execution;
- durable dependence on physical `EqClassId`;
- a correctness fallback whose branches implement different equivalence semantics.


## Structural retained-state comparison once encoded lanes already exist

For the external-class SAMF bridge, the old SAMF-local layer still retains:

```text
RevisionObservableCatalog
  definitions + reverse definitions
  class signature -> local EqClassId
  local EqClassId -> class record

CertifiedSemanticMorphism
  product-class -> projected local-class tuple

SupportAtomFabric
  signature -> atom
  atom -> rows/signature
  row -> atom
  projection incidence
```

The catalog-free fabric retains only the final support-fabric block. Atomic canonical payload and atomic class identity are already shared in `RevisionSemanticClassCatalog`; they are not copied into SAMF.

Thus, conditional on the encoded semantic substrate already being present, removing the SAMF-local catalog is a strict structural retained-state reduction: it deletes local observable definitions, local atomic wrappers, local product-class records, and product-projection mapping without deleting any row/fiber/projection information consumed by production execution.

This is stronger than a naming cleanup: the deleted maps encode information already derivable from `(binding, global atomic signature, fabric)`.

## Integration boundary

The production agent should treat the following invariant as the central contract:

```text
for every live SAMF row r:
  fabric.row_signature(r)
    == tuple(encoded_column_i.class_of(r))
```

and every class in that tuple must be live in the same candidate store's `RevisionSemanticClassCatalog` until the fabric mutation that removes its final row reference has completed.

This gives the mutation ordering:

```text
insert:
  retain/intern atomic classes
  update encoded lanes
  insert signature into fabric

remove:
  remove row from fabric
  update encoded lanes/postings
  release atomic classes
```

All steps occur on the same candidate `PhysicalStore` snapshot and publish atomically through the existing store transition mechanism.

## Release microprobe of the layer being removed

A temporary release-only probe compared the two SAMF-local constructions after global atomic semantic classes are already available.

Fixture:

```text
32,768 rows
512 joint atoms
arity = 2
100,000 repeated exact joint probes
```

Old external-class bridge per row:

```text
global EqClassId
 -> local observable wrapper EqClassId
 -> local product EqClassId
 -> SupportAtomFabric
```

Catalog-free path:

```text
global EqClassId tuple
 -> SemanticSupportFabric
```

Seven release runs:

```text
build ratio old/new: 1.302, 1.906, 1.754, 1.711, 1.782, 1.953, 1.733
probe ratio old/new: 3.042, 2.616, 3.176, 3.234, 2.938, 3.221, 3.031
```

The old probe includes the two local `lookup_semantic_class` translations that the current external-class SAMF requires after global semantic classes are known. The catalog-free probe consumes those global IDs directly.

This is a focused microprobe of the removable SAMF-local layer, not an end-to-end database benchmark. It supports the structural result: removing the local class namespace is not buying architectural simplicity by adding an obvious hot-path penalty; in this fixture the redundant translation layer is materially slower.

## Verification

Final source verification after the catalog-free fabric/test changes:

```text
kernel-semantics --lib: 103 passed / 0 failed / 1 ignored
kernel-plan revision_semantic_: 9 passed / 0 failed
kernel-semantics clippy -D warnings: PASS
kernel-plan clippy -D warnings: PASS
cargo fmt --all -- --check: PASS
```

The temporary release benchmark example was removed after execution and is not part of the R&D source delta.

## Repository-integrity note

`bash scripts/verify-repository.sh` was also run after functional freeze. It reports checksum mismatches for 15 source files. This is expected for the current accumulated R&D workspace: those files contain the accepted R&D deltas from the preceding semantic-substrate iterations and have not been re-sealed into the repository's production manifest. The verifier reported checksum mismatch only; this pass does not claim a production repository-manifest gate.

The R&D delta ZIP has its own `SHA256SUMS.txt` and passed ZIP integrity. Production integration must update the authoritative repository manifest only after selecting and integrating the accepted R&D changes.

## Wall clock

```text
R&D start:          23:13:12 UTC
functional freeze:  23:28:07 UTC
```

Functional source work stopped before the 20-minute useful boundary (`23:33:12 UTC`).
