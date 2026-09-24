# CFMD Rust implementation pass 02 — 2026-09-19

## Scope

This pass resumed the interrupted pass02 from the Library checkpoint `cfmd_workspace_pass02_checkpoint.zip` and stayed above physical storage. The target was semantic closure of relational/nested execution, equality modules, fixed-point/aggregate/lens scaffolding, and stronger revision pinning.

## Baseline checkpoint

- checkpoint SHA-256: `28aa619b2223b72b4c360c14ab61c298788c7d5760dd98a1df2fcde0b59d6839`
- interrupted pass02 baseline: 58 passing tests
- current workspace: 71 passing tests
- Rust source: ~5.3k LOC
- external crates: 0
- `unsafe`: 0
- ambient time/random/hash-container semantics in production crates: none found

## Problem → hypothesis → implementation → falsification → result

### 1. Relation semantics were not typed

**Problem.** `RelationDef` contained only columns. Every `Scan` returned a bag, so logical Set-vs-Bag semantics could disappear at the relation boundary.

**Hypothesis.** Set/Bag must be part of the relation type, not inferred from runtime rows.

**Implementation.** Added `RelationSemantics::{Set { column_equivalences }, Bag}` to schema. Set relations require one equivalence module per column. `Scan`, projection, filter and joins propagate Set/Bag semantics explicitly.

**Falsification.** Added tests for set scans and duplicate relation rows under case-insensitive semantic equality.

**Result.** Set and Bag are now distinct across schema, validation and relational query execution.

### 2. Γ pinning did not imply type-correct equality

**Problem.** An `I64Exact` module could be pinned as the equality for `Set<Text>` and survive validation if the collection had fewer than two elements.

**Hypothesis.** Every equality module requires an explicit semantic domain/signature checked independently of data cardinality.

**Implementation.** Added `EquivalenceDomain` and module-domain checking. Validation now checks collection/map/relation equality domains even for empty/singleton values.

**Falsification.** A singleton `Set<Text>` using `I64Exact` is rejected before pairwise comparison.

**Result.** Γ is no longer merely “module exists”; module kind must match declared data semantics.

### 3. Relational queries lacked compile-time semantic typing

**Problem.** `DISTINCT` or `FilterEqConst` over an empty relation could carry a wrong equality module or a constant of the wrong type and still execute successfully.

**Hypothesis.** Query correctness must not depend on whether runtime data happens to exercise an operator.

**Implementation.** Added `RelType`, `RelExpr::typecheck`, `PreparedRelExpr`, semantic-revision pinning, typed Filter/Project/Join/Distinct/Promote rules, equality-domain checking, constant shape checking, and join type compatibility.

**Falsification.** Empty-relation tests now reject wrong equality modules and wrong constant types. Join rejects unrelated `Ref<Person>` vs `Ref<Order>` even though both use entity-ID equality.

**Result.** Relational queries now have a real compile/prepare phase rather than relying on dynamic accidents.

### 4. Revision IDs could alias different semantics

**Problem.** `(SchemaRevisionId, SemanticEnvId)` are labels. Two structurally different contexts can reuse the same numeric IDs. Prepared queries and stored revisions previously compared only those IDs.

**Hypothesis.** Correctness must pin the immutable semantic context itself; content hashing/interning is a later storage optimization.

**Implementation.** `PreparedRelExpr` pins the complete `SemanticContext`. `Revision` retains the complete context, and storage transport checks compare actual contexts rather than revision numbers alone.

**Falsification.** Tests construct different schemas with identical revision numbers; prepared query reuse and snapshot commit are rejected.

**Result.** Reproducibility no longer trusts user-assignable revision numbers as semantic identity.

### 5. Pinned module digests could be unavailable at runtime

**Problem.** An empty database could commit a schema whose Γ referenced a digest not installed in the semantic registry; no value comparison happened, so the error stayed latent.

**Hypothesis.** A revision is valid only if every schema semantic dependency is executable now (or explicitly archived/certified later).

**Implementation.** Added `SemanticRegistry::validate_context` and storage commit enforcement.

**Falsification.** Empty set relation + pinned but missing equality implementation now fails bootstrap.

**Result.** A committed revision cannot promise unavailable semantics.

### 6. Algebraic scalar kernel was not closed

**Problem.** Runtime had `Value::Unit` but schema had no Unit type; Bool collections had no equality module. Exact f64 aggregation existed outside the model, so float was not representable in `(S, Γ, M)`.

**Hypothesis.** Base scalar domains must be explicit and law-indexed just like composite structures.

**Implementation.** Added `ScalarType::Unit`, Bool/Unit equality modules, `ScalarType::F64`, `Value::F64Bits(u64)`, and `F64Bitwise` equality. Float storage is bit-exact; IEEE `==` is deliberately not used as Set/Map equivalence because NaN violates reflexivity.

**Falsification.** Tests verify Unit is schema-typable and F64 NaN payload / signed-zero behavior follows the declared bitwise equivalence.

**Result.** The scalar layer is closer to algebraic closure and float semantics are explicit instead of ambient.

Built-in equivalence modules are additionally falsified against reflexivity, symmetry and transitivity on hostile samples (case folding, signed zero and NaN payloads), so the current registry does not merely *name* equality laws; it tests their minimum algebraic contract.

### 7. Strict lint failure exposed query-evaluator structure

**Problem.** Interrupted pass02 left `RelExpr::evaluate` and typecheck as monolithic functions.

**Implementation.** Split evaluator/typechecker into operator-level helpers and introduced an evaluation context; no clippy exemptions were added.

**Result.** `cargo clippy --workspace --all-targets -- -D warnings` passes.

## Existing pass02 components preserved and verified

- `kernel-semantics`: executable versioned equality modules.
- `kernel-validation`: full schema/model validation.
- `kernel-fixpoint`: reachability solver + independently checkable least-fixpoint certificate.
- `kernel-aggregate`: exact, reproducible finite-f64 summation with associative merge state.
- `kernel-lens`: Identity/ProductField/Compose with executable GetPut, PutGet and PutPut checks.

## Final checks

- `cargo test --workspace`: PASS, 71 tests.
- `cargo test --workspace --release`: PASS, 71 tests.
- `cargo clippy --workspace --all-targets -- -D warnings`: PASS.
- external crates: none.
- `unsafe`: none.

## Remaining architecture obligations

1. `Revision::new` in `kernel-model` can still be called directly without a `SemanticRegistry`; the trusted construction boundary should be moved to a revision/store layer rather than documented away.
2. Equality modules currently have scalar domains only. General `Set<Product<...>>` / map keys over structural values need certified composite equivalence signatures, not host equality.
3. Exact f64 accumulation is implemented but not yet exposed as a typed relational/group aggregate operator.
4. General certified schema/Γ transport remains absent; context changes are intentionally rejected rather than guessed.
5. `kernel-fixpoint` proves one solver family (reachability), not yet a generic certified-solver PlanIR boundary.
6. Writable lens complements are executable only for the current small lens calculus; compatibility-vault lifecycle is not implemented.
7. Persistent WAL/recovery, physical lowering, indexes, compaction and benchmarks remain intentionally untouched.

The next pass should resolve the trusted Revision construction boundary and structural equivalence signatures before any persistent storage format is frozen.

## Diff from pre-cycle checkpoint

The 15-minute continuation changed 11 tracked workspace files: approximately 1,369 insertions and 150 deletions, dominated by the relational typechecker/evaluator refactor and semantic validation. Final source-only workspace contains 16 crates and ~5.3k Rust LOC. Build cache was removed after verification.

