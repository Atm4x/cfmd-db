# CFMD Pass 03 — semantic boundary / structural equality / aggregates / transport

Date: 2026-09-19
Status: CLOSED for this wall-clock cycle

## Baseline

Pass02 final: 16 crates, 71 tests.
Pass03 final: 18 crates, 84 tests, ~6.4k Rust LOC, zero external crates.

Strict gate:

- `cargo fmt --all -- --check` — PASS
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS
- `cargo test --workspace` — 84/84 PASS
- `cargo test --workspace --release` — 84/84 PASS
- production `unsafe` — none
- production `HashMap` / `HashSet` — none
- production TODO/FIXME — none

Source-only delta from pass02: ~1332 insertions / 159 deletions across 16 files.

## 1. Trusted revision boundary

### Problem
`kernel-model::Revision::new` could construct a revision without forcing `SemanticRegistry` and full typed-state validation. The storage layer validated it, but the type itself did not encode the trust boundary.

### Hypothesis
Committed revision construction should live in a crate that depends on schema, model, semantic registry and typed validation. Raw model data must not be able to mint a trusted revision.

### Implementation
Added `kernel-revision`.

`Revision` fields are private and `Revision::build` always performs:

1. semantic-context structural validation;
2. runtime semantic-module resolution through `SemanticRegistry`;
3. lifecycle normalization;
4. full typed model validation.

`storage-memory` now stores only this validated revision type.

### Falsification
A model with an I64 field containing Text is rejected by revision construction itself.

### Result
Trusted revision creation is now registry-aware by construction, rather than a convention of the storage caller.

## 2. Structural equality without per-record plugins

### Problem
Primitive equality modules in Gamma handled scalar types only. Composite Set/Map keys would otherwise require either host-language equality or a custom external module for every record shape.

### Hypothesis
Primitive equality laws belong in Gamma, but purely structural equality should be a schema-derived composition of already-pinned laws.

### Implementation
Added `StructuralEquivalenceDef::Product` to Schema.

A product equality maps each semantic field ID to a child equivalence. `SemanticRegistry` resolves this recursively. Structural equivalences themselves are not external modules; their primitive leaves remain pinned in Gamma.

Added:

- recursive `EquivalenceDomain::Product`;
- cycle detection for structural equivalences;
- dependency expansion from structural law to primitive Gamma modules;
- validation of product equality against the actual `TypeExpr::Product` domain.

### Falsification
- case-insensitive Text + exact I64 composite keys collapse correctly;
- cyclic structural equality definitions are rejected before data exists;
- `Set<Product<...>>` rejects semantic duplicates;
- relational `DISTINCT` over a Product column uses the derived structural law end-to-end.

### Result
Composite record keys no longer require EAV-like flattening, Rust `Ord`, or a bespoke equality plugin.

## 3. Nominal entity references remain nominal at runtime

### Problem
`Value::LiveEntityRef(EntityId)` lost the entity type after typechecking. A literal reference of the wrong nominal type could therefore survive far enough to rely on surrounding schema assumptions.

### Hypothesis
Nominal identity must remain explicit in runtime values and equality modules.

### Implementation
References now carry both type and atom ID:

- `LiveEntityRef { entity_type, id }`
- `HistoricalEntityId { entity_type, id }`

Primitive equality is correspondingly typed:

- `LiveEntityIdExact(EntityType)`
- `HistoricalEntityIdExact(EntityType)`

The equality domain records the nominal target type.

### Falsification
- equality for `Person` rejects an `Order` runtime reference;
- a relational filter with an `Order` literal against `Ref<Person>` is rejected at prepare/typecheck time;
- existing lifecycle dangling-reference tests remain green.

### Result
Nominal identity now survives from schema through values, equality, queries and storage validation.

## 4. Exact F64 aggregation entered the typed query kernel

### Problem
`ExactF64Sum` existed as an isolated arithmetic crate; it did not prove that exact/reproducible float laws integrate with relational typing and Gamma equality.

### Hypothesis
An exact aggregate must be a typed query operator whose key equalities and output equality are explicit semantic inputs.

### Implementation
Added `RelExpr::GroupExactF64Sum`.

The prepare/typecheck phase verifies:

- group columns exist;
- every group key has a compatible equality law;
- the aggregate input is F64;
- the output F64 equality is pinned and type-correct.

Runtime uses `ExactF64Sum` and emits bit-exact `F64Bits`.

### Falsification
`1e16 + 1 - 1e16` in one group produces exactly `1.0`; NaN/Infinity are rejected explicitly.

### Result
Exact floating aggregation is now part of the same query semantics as Set/Bag/Join, not a side experiment.

## 5. Certified fixed-point solver boundary

### Problem
Reachability already emitted a certificate, but the API was concrete and did not express the general separation between logical fixed-point semantics and solver implementations.

### Hypothesis
Solver-specific algorithms should implement one small certified boundary; consumers should accept only checked certificates.

### Implementation
Added `CertifiedFixpointSolver`, `CheckedCertificate<C>` and `verify<S>()`. Reachability is now one implementation (`ReachabilitySolver`).

### Falsification
A certificate cannot become `CheckedCertificate` without passing the solver checker; existing nonleast/nonclosed hostile certificates remain rejected.

### Result
The kernel now has an executable shape for future Dijkstra/min-plus/etc. certified solvers without making them new logical primitives.

## 6. Definitional semantic transport

### Problem
Pass02 correctly rejected silent cross-(Schema,Gamma) commits, but even a pure rename/revision bump therefore had no executable legal path.

### Hypothesis
The first safe transport class should be definitional equivalence: identical semantic structure and pinned modules, allowing only presentation/revision differences.

### Implementation
Added `kernel-transport` and `DefinitionalTransport`.

Schema definitional equivalence ignores presentation names and revision labels but compares all semantic definitions, inclusions and structural equality laws. Environment definitional equivalence compares pinned module maps, not revision numbers.

`storage-memory` now records parent rewrites as either lifecycle edits or definitional transports. A transport contributes identity to lifecycle intent composition.

### Falsification
- rename + revision bump transports state without data rewrite;
- semantic field/type changes are rejected;
- an LCA merge can cross a verified rename edge while still composing lifecycle intents before GC.

### Result
Cross-schema revision is no longer either forbidden or unchecked: there is now a first small proof-carrying transport class.

## Bugs caught during the pass

1. Temporary typed-entity module digest encoder had an out-of-bounds write. A hostile query test exposed the panic; encoder was corrected and full tests rerun.
2. Broad `EntityIdExact` was semantically too weak; split into nominal live/historical equality modules.
3. Runtime refs lost nominal type; representation redesigned instead of patching operators.
4. Structural equality dependencies initially removed the derived ID without adding primitive child dependencies to Gamma; dependency expansion was corrected.
5. Structural equality needed explicit cycle rejection before any data path invokes it.
6. Lifecycle merge needed transport edges to behave as identity rewrites when reconstructing intent from LCA.

## Next attack surface

The remaining near-term executable work is now narrower:

1. richer structural equivalence constructors (`Option`, `Sum`, nested sequence/map laws where mathematically valid);
2. general non-definitional certified schema/data transport and identity transport;
3. Group/Aggregate IR beyond F64 sum (count, exact integer/decimal, law-indexed monoids);
4. fixed-point `PlanIR` integration rather than crate-local solver certificates;
5. lens complements / compatibility vault;
6. persistent WAL/recovery and physical lowering only after these semantic boundaries stay stable;
7. benchmarks for zero-abstraction-tax claims.
