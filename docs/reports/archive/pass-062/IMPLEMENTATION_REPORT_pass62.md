# IMPLEMENTATION REPORT — Pass62

Pass62 hostile-reviews and integrates the Program4 algebraic-native-layout R&D result onto authoritative Pass61. The integration is semantic/manual rather than a mechanical Pass60 patch merge.

## 1. Recursive algebraic native columns

`crates/kernel-plan/src/algebraic_native.rs` adds `AlgebraicNativeColumn`, an exact reconstructible physical representation for Product, Sum, Option, Seq, Set, Bag, Map and guarded Mu/Var. Scalar leaves reuse existing `NativeColumn` families. `NativeColumn::Algebraic` and `NativeRelation::typed_from_rows` integrate the family with ordinary typed columnar storage, projection, selection, append, swap-remove, schema validation and row materialization.

Logical `Value`, `TypeExpr` and pinned Γ remain authority. No algebraic layout is durable semantic state.

## 2. Compositional structural typed-batch predicates

The R&D branch's structural equality filter was generalized into `TypedBatchPredicateKind::{Primitive, Algebraic}`. A structural predicate stores one pinned-Γ `CanonicalEqKey` for the constant and compares candidate keys directly from algebraic storage.

This works through the existing typed-batch compiler rather than only one `Filter -> Project` fused pattern. Hostile coverage proves a structural filter can feed typed Group Count without materializing its full input relation as logical rows. The specialist I64 batch microkernel remains restricted to primitive I64 predicates.

## 3. Program3 coexistence

Pass61's `DenseLiveEntityIds` family is preserved. A mixed algebraic-structural predicate + dense-live-ref projection fixture survives an authoritative mixed remove/insert transition, preserving logical scan order and exact external-ID reconstruction.

Nested LiveRef leaves inside an algebraic structural value currently use external-ID scalar leaves. Automatic dense-local nested lowering is intentionally left to the broader multi-family layout advisor rather than hidden inside Program4 integration.

## 4. Authority/API correction

Low-level algebraic selection, append, swap-remove, acceptance and canonical-key methods are crate-internal. External mutation continues through the existing PhysicalStore candidate/stable-handle boundary.

## 5. Hostile recursion correction

The public algebraic constructor now validates `TypeExpr` before recursive descent. A non-empty unguarded `Mu { body: Var(self) }` is rejected instead of recursively expanding indefinitely. Internal mutation uses already-validated construction to avoid adding repeated type-validation tax to every append check.

## 6. Canonical-law differential test

A composed Product containing Option/Seq/Set/Bag/Map/Sum is canonicalized both directly from native storage and by `SemanticRegistry::canonical_equivalence_key` over the logical value. Exact key identity is asserted for every row, including case-different values equivalent under pinned TextAsciiCaseInsensitive Γ.

## 7. Verification/performance

All required Rust 1.98.1 gates pass on frozen source, including overflow-checked release workspace tests. Cold compilation timeouts were not counted; warmed standalone retries passed.

Five-process frozen release diagnostics:

- Product child-field native scan: median 41.834x vs boxed logical Product fixture, range 25.634–50.328x.
- Sum tag native scan: median 1.806x vs boxed logical Variant fixture, range 1.508–2.016x.

These are representation diagnostics, not universal performance claims.

Static snapshot: 401 declared tests, 151 kernel-plan tests, 21 crates, 55,476 Rust LOC, 0 unsafe, 19 pre-existing allow sites, 0 TODO/FIXME/todo!/unimplemented!, 0 GenericScan and 0 external Cargo sources.
