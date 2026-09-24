# CFMD Pass 07 — Relational sensitivity, semantic bags, proof-token hardening, ordering contracts

Date: 2026-09-19
Wall-clock target: 15 minutes
Baseline: pass06 (120 tests)
Final strict test count: 131 tests

## 1. Relational Impact now transports through identity isomorphisms

**Problem.** Pass06 proved coordinate invariance only for scalar `ExactQuery` impact. Relational queries still had no corresponding transport law.

**Hypothesis.** If an AtomId transport is bijective and both the model change and all entity-ref constants in `RelExpr` are transported, semantic impact must commute with the transport.

**Implementation.** Added universal relational derivative/impact by recomputation, transport of `FiniteModel`, `Change<FiniteModel>`, and recursive transport of `RelExpr`. Added a commuting-square falsifier for relational impact.

**Falsification.** A `Bag<Ref<Person>>` relation changes row `Person(1)` to `Person(2)` while a filter contains literal `Person(1)`. Under bijection `{1->11,2->12}`, query, old model and new model are all transported. Impact agrees in both coordinate systems.

**Result.** Relational sensitivity is coordinate invariant under verified identity isomorphism for the current relational IR.

## 2. Ordering became a third executable semantic-module kind

**Problem.** Equality and tokenizer modules demonstrated the contract/implementation split, but ordering/collation is independently observable semantics and will affect future `ORDER BY` / `TopK`.

**Implementation.** Added versioned ordering contracts:

- `I64Ascending`;
- `TextBinary`;
- `TextAsciiCaseInsensitiveThenBinary`.

Registry dispatch, implementation revisions, module availability and same-contract comparison use the same generic Γ machinery as equality/tokenizers.

**Falsification.** Same ordering contract with implementation revision v1 -> v2 is semantically equivalent. `TextBinary -> TextAsciiCaseInsensitiveThenBinary` is not. Existing semantic-environment transport accepts the former and rejects the latter without ordering-specific transport code.

**Result.** The Γ architecture is now exercised by three genuinely different module kinds.

## 3. Checked certificates were forgeable across checkers

**Problem.** `CheckedCertificate<C>` was parameterized only by certificate payload type. Another checker could validate the same payload type under a permissive rule and produce an object indistinguishable from one checked by the trusted checker.

**Implementation.** `CheckedCertificate` is now parameterized by the concrete `CertificateChecker` type. The constructor remains private and only `verify_certificate::<Checker>` can create the branded token.

**Result.** A token checked by checker A is a different Rust type from a token checked by checker B.

## 4. Checked certificates also failed to bind the specification value

**Problem.** Checker branding alone was insufficient. A certificate may be valid for spec A but not spec B; the old checked token forgot which spec value was checked.

**Implementation.** `CheckedCertificate<Checker>` now stores both the checked spec and certificate. `verify_certificate` clones and binds the actual spec value. Callers can inspect `checked.spec()` and `checked.certificate()`.

**Result.** The proof-admission boundary now binds `(checker, spec, certificate)`, not merely certificate payload.

## 5. First certified semantic-implementation admission path

**Problem.** Future external implementations must not be able to claim a semantic `contract_id` merely by metadata.

**Implementation.** Added `SemanticContract`, `SemanticImplementationArtifact`, `SemanticImplementationChecker`, `certify_implementation`, and registry installation from a contract-bound checked certificate. The current artifact IR is intentionally closed/builtin; it establishes the trusted admission path before arbitrary native binaries exist.

**Falsification.** A `TextAsciiCaseInsensitive` artifact cannot be certified against a `TextExact` contract. A correctly checked artifact installs at an implementation revision and produces the same digest as the direct builtin implementation.

**Result.** Contract claims now have a proof-token boundary. Native/JIT implementation equivalence still requires a stronger future checker/translation validator.

## 6. Bag relations require semantic row equality

**Problem.** Adding exact relational `Impact` exposed a correctness hole. `Bag` relations previously carried only rows, while `Set` carried column equivalences. Comparing two bag query results by Rust `Vec<Row>` representation is not semantic equality. Example: under case-insensitive text equality, bag row `"A"` and bag row `"a"` denote the same multiset element.

**Implementation.** `RelationSemantics::Bag` now carries `column_equivalences`, just like Set. Schema arity checks, semantic dependency collection, Γ validation, project/join/promote typing and relation transports preserve those laws.

`rel_impact_by_recompute` compares relation results as semantic multisets using the pinned equivalence laws, not host-language equality. Universal relational derivative uses the same semantic unchanged test.

**Falsification.** Replacing relation `{ "A" }` with `{ "a" }` under ASCII case-insensitive bag equality now yields `Impact::Unaffected` and `Change::NoChange`.

**Additional falsification.** The same Bag result is now checked under three hostile changes: row representative change within one equivalence class (`"A" -> "a"` under ASCII-CI) is `Unaffected`; physical row permutation is `Unaffected`; adding another equivalent occurrence changes multiplicity and is `Changed`. Universal relational derivative delegates its unchanged decision to this semantic Impact result.

**Result.** Relational sensitivity no longer depends on incidental row representation or row order, while multiplicity remains observable.

## 7. Structural equivalence now closes over collection values

**Problem.** Once Bag relations require column equality, a relation column may itself be `Set<T>`, `Bag<T>`, `Seq<T>` or `Map<K,V>`. The previous structural-equivalence calculus covered Product/Option/Sum only.

**Implementation.** Added derived structural equivalence for:

- Set — extensional unordered matching;
- Bag — unordered element matching plus multiplicity equality;
- Seq — positional equality;
- Map — extensional key/value matching.

Corresponding `EquivalenceDomain` forms and semantic dependency traversal were added.

**Falsification.** Hostile values with different physical order and ASCII case compare equal for sets/maps when the child law says so; bags also require equal multiplicities, while sequences remain order-sensitive.

**Result.** Relation-level Bag equality can now type nested collection columns without falling back to host equality.

## 8. Relational Impact across semantic-context transports

**Problem.** Pass06/pass07 already transported relational sensitivity through AtomId isomorphisms, but the same law had not been exercised across identity-like changes of `(Schema, Γ)`.

**Implementation.** Verified `DefinitionalTransport`, same-contract `EquivalentSemanticEnvironmentTransport`, and directional `ConservativeSemanticEnvironmentExtension` now expose a relational-Impact preservation check. The query must typecheck in both contexts; state/change are unchanged because these transports are identity rewrites on canonical data.

**Falsification.** A scan over `Text` changed from `"A"` to `"a"` remains `Changed` across a presentation-only schema revision, an implementation-only upgrade of `TextExact`, and a conservative Γ extension. In contrast, an explicit `TextExact -> TextAsciiCaseInsensitive` `SemanticLawMigration` changes the same Impact from `Changed` to `Unaffected`. No generic invariance theorem is exposed for genuine law migrations.

**Result.** Sensitivity transport is now aligned with the same boundary used by revision transport: only semantics-preserving context changes inherit Impact automatically.

## 9. Verification gate

- `cargo fmt --check` — PASS
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS
- `cargo test --workspace` — 131/131 PASS
- `cargo test --workspace --release` — 131/131 PASS
- external registry/git dependencies — 0
- production `unsafe` — 0
- production `HashMap` / `HashSet` — 0
- production `panic!/unwrap()/expect()` — 0
- TODO/FIXME — 0
- Rust source size — 11,310 LOC

## 10. Remaining real frontier

1. Fine-grained relational deltas instead of universal `Replace<FiniteModel>` fallback; correctness no longer depends on them, performance does.
2. Transport relational deltas/sensitivities through non-identity schema transports, not only AtomId isomorphisms.
3. Ordering-aware exact query operators (`OrderedView` / `TopK`) so ordering contracts become observable inside query IR rather than only registry API.
4. Strong certification/translation validation for native/JIT semantic-module implementations; the current checked artifact IR is intentionally closed.
5. Collection structural equivalence for recursive `μ` values requires a guarded recursive equivalence construction rather than ad-hoc recursion.
6. Nominal carrier/type transport and lineage-safe split/merge.
7. Lens complements / CompatibilityVault under retention/erasure laws.
8. Persistent fragments/WAL/recovery, then physical lowering and zero-abstraction-tax benchmarks.
