# CFMD Pass 05 — Semantic Environment / Relation / Identity Transport

Date: 2026-09-19
Wall-clock target: 15 minutes
Baseline: pass04-final (95 tests)
Final strict test count: 110 tests

## 1. Same semantic law vs implementation revision

**Problem.** `Γ` previously mapped semantic symbols directly to implementation digests. Any binary implementation change therefore looked indistinguishable from a change of logical semantics.

**Hypothesis.** Logical contract identity and implementation identity must be separate. A new implementation can be an identity semantic transport only when the registry certifies that both digests implement the same contract.

**Implementation.** `kernel-semantics` now stores `EquivalenceImplementation { contract, implementation_revision }`. `install_equivalence_revision` produces distinct implementation digests while `equivalent_implementation_contract` compares their logical contracts. Added `EquivalentSemanticEnvironmentTransport`.

**Falsification.** `TextExact(v1) -> TextExact(v2)` succeeds as identity state transport. `TextExact -> TextAsciiCaseInsensitive` is rejected as `SemanticContractChanged`.

**Result.** Binary/module upgrade and semantic law change are no longer conflated.

## 2. Genuine semantic-law migration

**Problem.** A real equality/collation change cannot be treated as a physical upgrade because it can alter uniqueness, grouping and query answers.

**Implementation.** Added explicit `SemanticLawMigration`. It clones semantic state only as a candidate and always rebuilds a trusted target `Revision`, forcing target validation under the new law.

**Hostile falsifier.** A Set relation containing `"A"` and `"a"` is valid under `TextExact` but invalid under case-insensitive equality.

**Result.** Migration fails with `DuplicateRelationRow`; no silent collapse or reinterpretation occurs.

## 3. Full Γ is query-visible semantics

**Problem.** Initial pass05 code compared only modules referenced by schema. But a prepared/ad-hoc query can name a module pinned in `Γ` even when schema does not reference it.

**Implementation.** `SemanticEnvironment::modules()` exposes immutable bindings. `SemanticRegistry::validate_context` now validates every pinned module, not only schema dependencies. Semantic context comparison covers all bindings.

**Falsification.** Empty schema + query-visible equality binding changing `TextExact -> TextCI` is detected as a law change.

**Result.** Historical replay semantics now corresponds to the whole pinned environment.

## 4. Conservative semantic-environment extension

**Observation.** Adding a new pinned module without changing any old binding is neither equivalence nor law migration. Old programs keep their meaning while new programs become expressible.

**Implementation.** Added `context_conservatively_extends` and `ConservativeSemanticEnvironmentExtension`. Direction matters: `Γ -> Γ + module` is transparent to existing semantics; reverse removal is not.

**Result.** Lifecycle branch merge may move toward the conservative extension but may not silently retract it.

## 5. Lifecycle merge path classification tightened

**Problem.** `lifecycle_intent_since` used to ignore every non-lifecycle revision edge. This was only valid for identity data rewrites.

**Implementation.** Parent rewrites are classified. The lifecycle intent collector transparently skips only definitional transport, same-contract semantic implementation transport, and conservative semantic extension. `TypedFieldTransport`, `TypedRelationTransport`, semantic law migration and identity remapping return `NonIdentityRewriteInLifecyclePath`.

**Result.** A schema/data/semantic rewrite cannot disappear from merge history merely because a later context happens to look compatible again.

## 6. Typed relation transport reuses query IR

**Problem.** Field migration already reused `ExactQuery`, but relation migration was missing and risked creating a second migration language.

**Implementation.** Added `RelationRewrite` and `TypedRelationTransport`. Target relation data is produced by the existing `RelExpr`, statically prepared against source `(S, Γ)`. Its complete `RelType` — columns plus Set/Bag semantics/equalities — must match the target relation definition. The materialized result then passes target revision validation.

**Falsification.** Bag source expression cannot populate a Set target unless query semantics explicitly produces a Set.

**Result.** Relational migration remains inside the same declarative calculus.

## 7. Prepared semantic query rebind

**Problem.** A semantic query prepared under implementation digest v1 was rejected after a same-contract v2 upgrade even though logical semantics had not changed.

**Implementation.** `PreparedRelExpr::rebind_preserving_semantics` re-typechecks against a target context only when `Γ_target` conservatively preserves the source semantics.

**Boundary.** This applies to semantic IR only. Future physical/JIT plans still require separate recertification because implementation ABI/layout may have changed.

## 8. Bijective AtomId transport

**Problem.** The theory claimed identity-preserving schema versions form a groupoid, but identity transport had not been exercised against an actual complete database state.

**Implementation.** Added `BijectiveIdentityRevisionTransport`. A total bijection over current live AtomIds coherently rewrites:

- lifecycle entities, roots and `KeepsAlive` edges;
- all carrier memberships;
- field owners;
- nested `LiveEntityRef` values;
- retained `HistoricalEntityId` values when their ID is in the transported domain;
- relation values recursively through Product/Option/Sum/Seq/Set/Bag/Map.

A target trusted revision is rebuilt afterward.

**Falsification.** Forward mapping `{1,2}->{11,12}` followed by the inverse produces the exact original `DatabaseState`.

**Important boundary.** Although the mapping is an identity isomorphism semantically, it is not an identity edge for the current lifecycle merge algorithm because intents still use old coordinates. Store history therefore blocks automatic lifecycle merge across this edge until intent conjugation is implemented.

## 9. Verification gate

Final checks:

- `cargo fmt --check` — PASS
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS
- `cargo test --workspace` — 110/110 PASS
- `cargo test --workspace --release` — 110/110 PASS
- external crates — 0
- `unsafe` — 0
- production `HashMap` / `HashSet` — 0
- TODO/FIXME — 0
- Rust source size — ~9.2k LOC across 18 crates

## 10. Next real frontier

1. Transport lifecycle/query/change intents through bijective identity maps (conjugation), then prove merge commutation.
2. General semantic-module registry kinds beyond equality: timezone/collation/tokenizer/certified function contracts.
3. Contract proof objects for external implementations; the current PoC's implementation revision cannot change behavior because execution still dispatches through the built-in contract enum.
4. Relation/carrier schema transformations involving nominal type changes and explicit lineage for split/merge.
5. Integrate fixed-point certificates into common PlanIR/proof checker.
6. Writable lens complements / compatibility vault with retention labels.
7. Only after these: persistent fragments/WAL/index lowering and zero-abstraction-tax benchmarks.

## 11. Diff / reproducibility audit

Relative to pass04-final, pass05 changes 11 tracked files (including reports/lockfile). The code changes are concentrated in `kernel-identity`, `kernel-query`, `kernel-schema`, `kernel-semantics`, `kernel-transport`, and `storage-memory`. `Cargo.lock` still contains only the 18 local workspace crates; no registry/external package was introduced.
