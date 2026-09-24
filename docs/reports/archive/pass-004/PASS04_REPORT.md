# CFMD Pass 04 — semantic closure + first non-definitional transport

## Scope

Pass 04 continued from `cfmd_workspace_pass03_final.zip`. The goal was to close the next semantic gaps before starting persistent storage: structural equivalence beyond products, reusable law-indexed aggregation, and a real typed semantic transport that reuses the normal query calculus rather than introducing a migration-only language.

## 1. Structural equivalence now closes over Option and Sum

### Problem
`StructuralEquivalenceDef` only supported `Product`. The type kernel already had `Option` and closed `Sum`, so equality was not compositionally closed across the algebraic type language.

### Hypothesis
Only primitive equality laws should live in Γ. Equality of algebraic values should be derived from the equality laws of their components.

### Implementation
Added structural equivalence definitions for:
- `Option { inner }`
- `Sum { variants }`

`SemanticRegistry::equivalence_domain` and `equivalent` now derive domains and compare values recursively. Schema dependency expansion recursively resolves the primitive Γ laws beneath structural equality.

### Falsification
Hostile tests cover:
- `Some("A") == Some("a")` under case-insensitive Text equality;
- `None != Some(...)`;
- equal Sum variants use their variant law;
- different Sum tags are never equal;
- cycles in derived equality remain rejected.

### Result
No new semantic plugin or storage primitive was required.

## 2. Query literals now respect the complete algebraic value shape

### Problem
The query typechecker accepted scalar constants only. A valid `FilterEqConst` on `Product`, `Option`, `Sum`, collection, or recursive values failed despite the schema and equality layers supporting those types.

### Hypothesis
Runtime-shape checking must be structurally recursive over the same algebraic type grammar as the schema.

### Implementation
Replaced scalar-only literal checking with recursive shape checking for:
- Product
- Option
- Sum
- Seq
- Set
- Bag
- Map
- guarded `μ` / `Var`

Added typed constants for otherwise ambiguous values such as `None` and empty collections.

### Falsification
A structural Option constant is now usable in a typed filter with schema-derived equality. An unannotated `None` remains rejected as ambiguous; a typed `None` is accepted.

### Result
Algebraic types are now compositional through schema → equality → query preparation.

## 3. ExactQuery acquired a reusable type checker

### Problem
A non-definitional schema transport needs a total pure transformation, but introducing a dedicated migration-expression language would duplicate semantics and create another trusted surface.

### Hypothesis
The same `ExactQuery` IR used for application computation should typecheck as a value transformation and be reusable for migration.

### Implementation
Added `ExactQuery::typecheck(input_type)` with typed rules for Input, constants, ProductField, SeqLength, SeqSumI64, AddI64, and If. Ambiguous constants require explicit type annotation.

### Result
Migration no longer needs an independent expression language.

## 4. First real non-definitional typed transport

### Problem
Pass03 only had `DefinitionalTransport`, which can rename/re-present an unchanged semantic model but cannot transform data.

### Hypothesis
A first general semantic-change boundary can be expressed as field rewrites whose value transformation is an ordinary typed `ExactQuery`.

### Implementation
Added `TypedFieldTransport` and `FieldRewrite`:
- source and target contexts are registry-validated;
- unchanged fields pass through;
- changed target fields require an explicit source field + typed ExactQuery;
- field owner nominal types must agree;
- query output type must exactly equal target field type;
- extra/unknown target rewrites are rejected;
- source-only fields are intentionally dropped;
- target state is fully validated again.

Crucially, transport is `Revision → Revision`, not raw `State → State`. It cannot bypass the trusted revision constructor.

### Falsification
Tests cover a real semantic rewrite (`I64 field -> new semantic field` through `+1`) and reject a transform whose output type disagrees with the target field.

### Result
This is the first non-definitional migration path and it reuses the existing computation semantics.

## 5. Γ changes cannot hide inside data transport

### Problem
The first TypedFieldTransport draft allowed source and target semantic environments to differ. A field could therefore pass through unchanged while its equality/collation semantics silently changed, provided the current data happened to remain valid.

### Hypothesis
Changing Γ is itself a semantic transport and must never be smuggled inside a field/data migration.

### Implementation
TypedFieldTransport now requires definitionally equivalent semantic environments. A Γ change returns `SemanticEnvironmentChangeRequiresTransport`.

### Falsification
A `Set<Text>` transported from TextExact to TextAsciiCaseInsensitive is rejected even when no duplicate data exists.

### Result
Validation of current data can no longer masquerade as proof of semantic equivalence.

## 6. Transport is now a first-class revision edge in MemoryStore

### Implementation
Added `commit_typed_field_transport`. The store:
- verifies the parent revision/context;
- executes the trusted transport;
- stores the returned validated target revision;
- records a typed transport edge in the DAG.

Lifecycle merge across arbitrary semantic transport is deliberately *not* auto-enabled yet; the existing merge path still requires a safe common semantic interpretation.

## 7. Aggregate IR was refactored around laws, not one node per function

### Problem
`GroupExactF64Sum` would lead to a separate query node for every aggregate.

### Hypothesis
Grouping is one operation; the aggregate should be an explicit algebraic specification with its own result type/equality and merge state.

### Implementation
Replaced the specialized node with:
- `RelExpr::Group`
- `AggregateSpec::Count`
- `AggregateSpec::ExactF64Sum`

`ExactCount` and `ExactF64Sum` are mergeable states. `ExactCount` uses the same arbitrary-precision integer core as the exact floating accumulator, so the accumulator itself has no hidden `u64/u128` bound; only conversion to the declared `I64` result can overflow explicitly.

### Falsification
Tests verify:
- semantic grouping under case-insensitive equality;
- Count results;
- exact F64 cancellation;
- nonfinite exact-sum rejection;
- empty global aggregation returns the aggregate identity (`count=0`) instead of zero rows.

### Result
Aggregate growth is now a data/law extension rather than query-IR proliferation.

## 8. Empty set relations exposed a context-validity bug

### Problem
A Set relation with a wrong equality domain could be committed if it had zero or one row. Duplicate checking never invoked the comparator.

### First attempted fix
Validate relation equality in `validate_state`.

### Falsification
The hostile empty-relation test still failed because an empty relation need not appear in `FiniteModel` at all.

### Correct fix
Equality-domain compatibility of relation columns is a `(Schema, Γ)` well-formedness property, not a data invariant. `SemanticRegistry::validate_context` now validates every Set relation's column equality against its declared column type before any revision exists.

### Result
Cardinality can no longer hide an invalid semantic context.

## Verification status

At the pass04 strict gate:
- 95 tests pass in release mode;
- `cargo clippy --workspace --all-targets -- -D warnings` passes;
- `cargo fmt --check` passes;
- external crates: 0;
- production `unsafe`: 0;
- `HashMap` / `HashSet`: 0;
- TODO/FIXME: 0.

## Next actual gaps

1. General semantic-environment transport for law/version changes. It must distinguish implementation replacement with identical semantics from real reinterpretation/coarsening/refinement.
2. Transport of relations, carriers and identities, not only fields.
3. More structural equivalences (Seq/Set/Bag/Map where mathematically lawful).
4. More AggregateSpec instances and a common certified aggregate-law boundary.
5. Unify certified fixed-point solver certificates with the main PlanIR proof checker.
6. Compatibility complements / writable schema lenses.
7. Only after those: persistent WAL/recovery and physical lowering benchmarks.
