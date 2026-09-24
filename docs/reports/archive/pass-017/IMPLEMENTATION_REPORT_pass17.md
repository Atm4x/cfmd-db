# CFMD Rust implementation pass 01

## Status

Rust 1.98.1. Dependency-free workspace. Full workspace passes:

- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo build --workspace --release`

Current unit/integration test count: 42.

## Workspace

- `kernel-types` — stable entity/schema/environment/revision IDs.
- `kernel-schema` — constructive structural type expressions, guarded `μ`, open capabilities, thin subtype order, semantic-module dependencies.
- `kernel-model` — structural values, nominal carriers, live/historical references, model-wide lifecycle normalization.
- `kernel-change` — universal replacement change plus fine set/sequence changes.
- `kernel-query` — closed total/deterministic query expression IR; universal recompute derivative and one checked fine derivative.
- `kernel-lifecycle` — roots + `KeepsAlive`, deterministic reachability normalization, semantic branch intents.
- `kernel-identity` — bijective identity transports and composition; split/merge rejected as transports.
- `kernel-retention` — explicit and control-flow retention-label propagation without arbitrary host callbacks.
- `kernel-proof` — tiny PlanIR fragment plus structural certificate checker for filter fusion.
- `storage-memory` — revision DAG, multi-LCA detection, intent-preserving lifecycle branch merge.
- `kernel-integration` — vertical cross-layer falsification test.

## Implementation findings that changed the design

1. Returning `&Revision` from `commit` held a mutable store borrow too long. Commit now returns stable `RevisionId`; revisions are fetched separately.
2. A revision DAG may have multiple lowest common ancestors. The store returns the complete LCA set and refuses to guess a unique merge base.
3. Storing only lifecycle-normalized branch snapshots loses information required by a later merge. The store now persists semantic lifecycle intents relative to the parent and reconstructs merge meaning from the LCA before normalizing once.
4. A hostile merge test attempted to revive an entity absent from the LCA. That is invalid: entity creation is a separate semantic rewrite. The corrected hostile case keeps the alternative owner alive in the LCA.
5. Generic Rust `Fn` as an `ExactQuery` violates the total/pure/extensional/deterministic query law. Query execution now uses a closed declarative expression IR; logical errors are deterministic `Result` values.
6. Normalizing only `LifecycleGraph` left dead entities in carriers/fields/relations. Model normalization now restricts the complete finite model and rejects dangling live references.
7. `LiveEntityRef` and historical identity were previously conflated. They are now separate value forms; historical IDs may outlive an entity, live references may not dangle.
8. `BTreeSet<Value>`/`BTreeMap<Value, ...>` silently made Rust equality/order part of database semantics. Logical `Set`, `Bag` and `Map` now carry explicit semantic equivalence IDs; relation storage no longer deduplicates through host-language ordering.
9. Schema semantic dependencies are now executable invariants: Set/Bag/Map equivalence modules must be pinned in `Γ` before a revision can be committed.
10. Lifecycle-only rewrites cannot silently cross `(Schema, Γ)` revisions. Until certified schema/environment transport exists, the store returns `SemanticTransportRequired`.
11. Generic closures in retention tracking could capture untracked external secrets. IFC primitives are now closed operations; control dependency (`pc`) is explicitly propagated by `select`.
12. Fine sequence-delta optimization is checked against the universal replacement derivative, preserving the rule that fine changes are only performance refinements.
13. The first exact-query evaluator cloned the whole input even for `SeqLength(Input)`, turning an O(1) logical operator into O(n). Internal evaluation now carries borrowed/owned values and materializes only when ownership is semantically required. This is the first direct zero-abstraction-tax bug found by implementation.
14. Schema-level references now distinguish live references from historical IDs, matching the runtime value layer instead of re-conflating the two concepts during type checking.

## Falsification coverage

- exhaustive lifecycle-normalization idempotence over all 3-node root masks and directed edge masks;
- non-conflicting lifecycle intent commutativity;
- conflicting fact edits produce typed conflicts;
- hostile LCA merge where one branch locally collects an entity and the other concurrently provides a new live path;
- criss-cross revision history with multiple LCAs;
- identity transport groupoid composition and inverse;
- split/merge rejected as identity transport;
- guarded recursive structural values accepted, naked recursive variables rejected;
- subtype diamonds accepted, subtype cycles rejected;
- semantic environment dependencies enforced;
- live vs historical reference lifetime behavior;
- universal derivative from-scratch law;
- fine `SeqLength` derivative checked against universal recomputation;
- implicit retention flow through branch control;
- PlanIR rewrite certificate accepted only for its exact structural rule;
- cross-layer integration scenario.

## Not implemented yet

These are deliberate missing layers, not hidden fallbacks:

- general certified schema / semantic-environment transport;
- model-wide semantic rewrites beyond lifecycle facts;
- relational/nested query algebra over the full finite model;
- fixed-point solver interface and certificates;
- equality/collation module execution (only versioned dependency plumbing exists);
- exact/reproducible floating aggregation;
- writable schema lenses and complement storage;
- approximate-query contract language;
- persistent/WAL storage, recovery, physical lowering, indexes and compaction;
- formal proof assistant mechanization;
- performance benchmarks.

The next implementation pass should attack the relational/nested query IR and semantic equality-module interface before adding physical storage. Otherwise storage would freeze unresolved logical semantics into a layout too early.

---

# Pass 02 update — 2026-09-19

Pass02 resumed from a Library checkpoint and raised the workspace from 58 to 71 passing tests. Full details are in `PASS02_REPORT.md`.

Major changes: relation-level Set/Bag semantics; typed semantic equality domains; compile-time relational query typing and prepared queries; full semantic-context pinning instead of trusting numeric revision IDs; commit-time availability checks for Γ modules; Unit/Bool/F64 scalar closure; bitwise-total F64 key equality; and strict refactoring of query evaluator/typechecker without lint suppressions.

All workspace debug/release tests and strict clippy pass. The implementation found no need for a new storage/data primitive in this pass. The principal unresolved semantic architecture issue is now the trusted revision-construction boundary plus general structural equivalence modules.

# Pass 03 addendum — trusted revisions, structural equality, typed aggregate, transport

Pass03 moved four previously documentary invariants into executable boundaries:

- trusted revisions are created only by `kernel-revision::Revision::build`, which requires registry + typed validation;
- composite equality is schema-derived from Gamma-pinned primitive laws, with cycle/domain checks;
- nominal entity type is retained in runtime references and entity equality modules;
- exact F64 grouping is a typed relational operator;
- fixed-point results cross a `CheckedCertificate` boundary;
- definitional schema/environment transport is a verified identity rewrite and participates in revision DAG history.

Strict final gate: 84/84 tests in debug and release, fmt/clippy clean, no external crates or unsafe.

See `PASS03_REPORT.md` for problem→hypothesis→implementation→falsification→result details.

## Pass 04 — algebraic closure, generic aggregates, typed semantic transport

Pass04 raised the executable test count from 84 to 95 and closed several cross-layer gaps without introducing new crates or storage primitives.

Key implementation changes:
- derived structural equality now covers Product/Option/Sum;
- recursive literal/type-shape validation covers the algebraic value family including guarded μ;
- `ExactQuery` now has a reusable type checker and typed constants;
- `TypedFieldTransport` reuses ExactQuery instead of defining a migration DSL;
- semantic transport is Revision→Revision and cannot bypass trusted validation;
- field transport forbids hidden Γ changes;
- MemoryStore records typed field transport as a revision edge;
- grouping is refactored to `Group + AggregateSpec` with Count and ExactF64Sum;
- Count uses arbitrary-precision internal state;
- aggregate identity semantics for empty global grouping is tested;
- relation equality-domain validation moved from data validation to `(Schema, Γ)` context validation after an empty-relation hostile test disproved the first placement.

Strict gate: 95/95 tests, release PASS, clippy `-D warnings` PASS, fmt PASS.

# Pass 05 — 2026-09-19

Implemented and falsified semantic-environment transport, conservative Γ extension, law migration, typed relation transport, semantic-query rebind, and whole-state bijective AtomId transport. The key correction was that all pinned Γ bindings are query-visible semantics, not only modules referenced by schema. Lifecycle merge history now rejects all non-coordinate-identity rewrite edges instead of silently skipping them. Final gate: 110 tests, debug/release PASS, clippy `-D warnings` PASS, fmt PASS, zero external crates, zero unsafe.

See `PASS05_REPORT.md` for the problem→hypothesis→implementation→falsification→result breakdown.

# Pass 06 addendum — identity-coordinate conjugation and generic proof/module boundaries

Pass06 raised the workspace from 110 to 120 tests. The central correction is that a bijective identity rewrite is now executable across branch history instead of being a permanent merge barrier. Identity proof-domain is retention-scoped and may include locally GC-pruned atoms; lifecycle graph/intents are conjugated through the bijection; LCA canonical data is transported before final merge so a concurrently rescued atom recovers carrier/field state rather than only an ID.

Merge no longer chooses an identity coordinate system by left/right convention. If `(S, Γ)` does not uniquely determine it, the default API returns `AmbiguousIdentityCoordinate`; an explicit revision-space merge API selects the intended branch coordinate system.

`Impact`/`Change<Value>` and `ExactQuery` now have identity-transport checks, including query constants. `SemanticRegistry` gained a second executable module kind (versioned tokenizer), and existing semantic-environment transport worked unchanged across it, validating the generic contract/implementation split. Wrong module kind is now distinguished from missing historical module.

Finally, `CheckedCertificate` and `CertificateChecker` were centralized in `kernel-proof`; fixed-point and optimizer rewrite certificates now cross one generic proof-admission boundary.

Strict final gate: 120/120 release tests, fmt/clippy clean, no external crates, no unsafe.

Pass06 final static audit: 0 panic-shaped calls in production sections before test modules; Cargo.lock contains 18 local workspace packages and no registry/git source. Rust source size is 10,018 LOC.

# Pass 07 — 2026-09-19

- Added relational derivative/Impact recomputation and AtomId-isomorphism transport for models, model changes and `RelExpr`.
- Added ordering as a third versioned Γ module kind.
- Hardened `CheckedCertificate` to bind both concrete checker type and checked spec value.
- Added the first contract-bound semantic implementation certification/admission path.
- Corrected relational Bag semantics: bags now carry per-column semantic equivalences.
- Corrected relational Impact/derivative to compare semantic multisets instead of Rust row representation.
- Extended structural equivalence to Set/Bag/Seq/Map.
- Strict gate: 131 tests in debug/release, full clippy/fmt pass, no external crates, no unsafe.

## Pass 07 final addendum — semantic Bag equality and context-transported sensitivity

Problem -> relational Bag results were still representationally compared in sensitivity paths, despite Bag being a mathematical multiset over pinned equivalence classes.

Hypothesis -> give Bag relations the same explicit column equivalences as Set, derive nested collection equality compositionally, and make relational Impact compare semantic multisets rather than `Vec<Row>`.

Implementation -> `RelationSemantics::Bag { column_equivalences }` is now threaded through schema dependencies, validation and query typing. `rel_impact_by_recompute` uses semantic row equality with one-to-one multiset matching. Identity-like semantic-context transports expose an Impact commuting check.

Falsification -> row permutation and ASCII-CI representative substitution are `Unaffected`; multiplicity change is `Changed`; `TextExact -> TextCI` law migration changes Impact and is deliberately excluded from the preservation theorem.

Result -> relational rebase/sensitivity no longer observes physical row order or representative bytes. Final pass07 gate: 131/131 debug and release tests, 18 crates, 11,310 Rust LOC, zero external crates/unsafe/production HashMap-HashSet/TODO-FIXME/panic-shaped calls.

# Pass 08 source checkpoint — ordering congruence and semantic relational deltas

Pass08 added exact `TopKWithTies`, explicit ordering/equality congruence checks, a total F64 ordering contract, and a semantic `RelationDelta` recomputation oracle bound to its full `RelType`. A hostile representative-dependence case (`TextBinary` ordering over ASCII-CI equality) is rejected during query preparation rather than allowed to leak stored representative bytes into query semantics.

Important verification note: the container was restored without the previously installed Rust toolchain. The source contains 136 declared tests, but pass08 has **not** been compiled or run. Pass07 remains the last fully verified checkpoint (131/131). See `PASS08_REPORT.md`.

# Pass 09 final addendum — executable pass08, representative-safe equality, incremental deltas, checked ordering laws

Problem -> restoring Rust exposed concrete pass08 gate failures and then a deeper semantic mismatch: equality-based query operators could observe representatives that relation equality declared interchangeable. The pass08 delta oracle also had no fast path, and ordering compatibility was still a runtime builtin table.

Hypothesis -> first make pass08 executable, then require query equality to be invariant under input equality, derive only those incremental rules that can be checked against the recomputation oracle, and move ordering/equality compatibility behind the existing checked-certificate trust boundary.

Implementation -> fixed compile/fmt/clippy issues; added compositional `equivalence_refines`; enforced it in Filter/Join/Distinct/Group; added `rel_delta_optimized` for Scan, Filter and Bag Project with conservative fallback; added semantic `RelationDelta` equivalence; added checked ordering-compatibility certificates and registry admission.

Falsification -> hostile representative substitutions, finer-equality filters/joins/distinct/groups, Bag projection replacement cancellation, a 64-pair old/new state-space differential test, and a Set projection support-count counterexample. The latter forces fallback instead of an unsound fast rule.

Result -> final gate is 145/145 declared tests in debug/release, strict Clippy/fmt clean, release build clean, 18 crates, 12,820 Rust LOC, zero external Cargo sources/unsafe/TODO-FIXME/panic-shaped macros. See `PASS09_REPORT.md`.

# Pass 10 verified checkpoint — support-count Set IVM and compositional local replay

Pass10 closes the previous `ΔProject(Set)` correctness fallback with semantic support counts, adds support-count `Distinct`, direct `PromoteToBag` delta transport, and compositional operator-local replay for `JoinEq` and `TopKWithTies`. Semantic child deltas can now be replayed into old intermediate relation values with consistency checks; Join/TopK recalculate only their operator node instead of evaluating changed child trees from the new model.

Hostile falsification now includes: 64-state Set projection, 64-state Distinct, 256 two-sided Join transitions, 64 TopK threshold/tie transitions, and a 256-transition composed `Project -> Distinct -> Join -> PromoteToBag` pipeline. Every optimized result is checked against `rel_delta_by_recompute` using semantic delta equivalence.

Final pass10 gate: 152 declared tests; 13,675 Rust LOC; debug/release tests PASS; release build PASS; rustfmt PASS; strict workspace Clippy `-D warnings` PASS. Remaining immediate semantic/incremental frontier: guarded recursive `μ` equivalence; `Group` delta; persisted support/provenance state; true indexed `ΔJoin` and maintained TopK order-statistics. OrderedView/pagination and physical lowering/abstraction-tax benchmarks remain larger follow-on work. See `PASS10_REPORT.md`.

# Pass 11 verified checkpoint — Group delta closure and guarded recursive μ equivalence

Pass11 closes both immediate correctness holes left by pass10. `Group` is now part of the compositional relational derivative tree via operator-local replay, with differential falsifiers for semantic group birth/death, empty global Count identity, and ExactF64Sum. Every current `RelExpr` constructor therefore has a correctness-supported derivative path.

Guarded recursive equality is now explicit rather than represented by arbitrary graph cycles. `StructuralEquivalenceDef::{Mu, Var}` and canonical `EquivalenceDomain::{Mu, Var(depth)}` make recursive type/equality domains comparable without binder-name identity. Structural-domain validation distinguishes free recursion, unguarded recursion, and illegal ordinary cycles; recursive equality evaluation terminates on finite recursive values; recursive refinement is checked through paired lexical binders. A vertical relational filter test exercises guarded μ type validation, recursive equality-domain matching, literal shape checking, query preparation and runtime equality together.

Final pass11 gate: 159 declared tests; 14,466 Rust LOC; debug/release tests PASS; release build PASS; rustfmt PASS; strict workspace Clippy `-D warnings` PASS; zero external Cargo sources/unsafe/TODO-FIXME/panic-shaped macros. One `clippy::too_many_lines` suppression exists only on the large recursive integration-style test fixture.

Immediate frontier is no longer missing relational-delta correctness or recursive equality. It is maintained incremental state (support/provenance, exact aggregate state), true indexed ΔJoin/order-statistics TopK, OrderedView/pagination, and then physical lowering/abstraction-tax benchmarks. See `PASS11_REPORT.md`.

# Pass 12 verified checkpoint — checked PlanIR, native-layout preservation, materialized Set support state

Pass12 adds a separate `kernel-plan` compiler layer rather than mixing physical policy into `kernel-query`. Every current relational logical operator has a closed PlanIR representation with explicit baseline algorithms. Lowering is admitted through `CheckedCertificate`: exact logical round-trip and one-to-one logical/physical node count are checked before a plan is trusted. `PreparedPlan` first performs normal query typechecking, pins the full semantic context, then carries the checked physical artifact.

A `PhysicalCatalog` can bind a relation to row, columnar, key-value, adjacency, dense-array, inverted or custom layouts. The binding survives checked lowering directly in the scan node, with no mandatory layout-conversion PlanIR node. A vertical `kernel-integration` test checks schema/Γ/model -> typed catalog-bound plan -> reference result equality. This removes a mandatory conversion from the representation contract but is not yet a runtime performance result.

Pass12 also introduces `MaterializedSetSupportState`: validated/context-bound semantic support counts with atomic delta application. Set projection and Distinct now share this state transition law, and exhaustive 64-transition tests for both operators agree with the recomputation oracle. The compatibility derivative API still rebuilds state per call; long-lived runtime ownership remains future work.

A release-only compiler diagnostic reports median ~519 ns for a 9-node lower+round-trip and ~1319 ns for a typed 3-node prepare+checked lowering across five final runs, with visible noisy outliers. This is compiler overhead only, not a runtime abstraction-tax benchmark.

Final pass12 gate: 171 declared tests; 19 crates; 15,718 Rust LOC; debug/release tests PASS; release build PASS; rustfmt PASS; strict workspace Clippy `-D warnings` PASS; zero external Cargo sources/unsafe/TODO-FIXME/panic-shaped macros. Main frontier: real native-layout physical executor and runtime abstraction-tax benchmarks, then persisted runtime state ownership, true indexed ΔJoin, maintained TopK/Group state, OrderedView, and durability engine layers. See `PASS12_REPORT.md`.

# Pass 13 verified checkpoint — native physical runtime, compiled primitive semantics, long-lived derivative state

Pass13 crosses the first real execution boundary. `kernel-plan` now owns `NativeRelation`/`PhysicalStore` and `PreparedPlan::execute_native`; physical execution is implemented for Scan, FilterEqConst, Project, Distinct, baseline NestedLoop JoinEq and PromoteToBag. Columnar Scan→Filter→Project has a fused path that reads only predicate values plus projected values for matching rows. Group and TopK deliberately return `UnsupportedPhysicalPlan`; there is no hidden logical-evaluator fallback.

The first runtime benchmark falsified an optimistic abstraction-tax expectation: the initial native loop was ~5.77x slower than a hand-written columnar loop because every row re-resolved semantic equality through Γ/registry. Primitive equality is now resolved once as `ResolvedPrimitiveEquivalence`, and constant predicates are compiled to `BoundPrimitivePredicate`. The final 9-round release benchmark is 605,310 ns native median versus 401,565 ns hand-written median on 100k rows (1.507x), with exactly 106,250 physical value reads for 100k predicates + 6,250 matches. The large lookup tax is gone, but the remaining ~50% gap is explicitly open.

Pass13 also adds `MaterializedRelDeltaState`, making support-count state a query/context-bound long-lived derivative object for Project(Set) and Distinct. Sequential multi-transition hostile tests carry one state instance across model changes and match the recomputation oracle at every step.

Final gate: 179 declared tests; 19 crates; 17,183 Rust LOC; debug/release tests PASS; release build PASS; rustfmt PASS; strict workspace Clippy `-D warnings` PASS; strict rustdoc PASS; zero external Cargo sources/unsafe/TODO-FIXME/panic-shaped macros. Remaining immediate frontier: close the measured physical-runtime gap, implement physical Group/TopK and non-row-materializing general column batches, then indexed ΔJoin and maintained Join/TopK/Group state. See `PASS13_REPORT.md`.

# Pass 14 verified — typed physical I64 kernel and complete current RelExpr physical coverage

Pass14 introduced schema-checked `I64Columnar` storage and a raw-I64 fused filter/project kernel while retaining exact logical `Value` output. On the final 100k-row benchmark, three independent process-level median ratios versus a hand-written typed baseline were 0.999x, 1.159x and 1.020x (median ratio 1.020x), reducing the Pass13 1.507x median gap to near-baseline for this one microkernel. This is diagnostic evidence, not a universal no-tax claim.

`PreparedPlan::execute_native_pinned` now executes directly against its owned immutable semantic context; the old explicit-context API remains as a compatibility validation boundary. Physical Group now supports Count and ExactF64Sum, and physical TopKWithTies supports exact threshold ties through semantic ordering. All current RelExpr operator shapes therefore have a native physical correctness path, although Join/Group/TopK remain baseline algorithms rather than performance-grade indexed/maintained implementations.

Final Pass14 gate: 184 declared tests, debug/release workspace tests PASS, strict Clippy/fmt/release build/rustdoc PASS, 19 crates, 17,869 Rust LOC, zero external Cargo sources, zero unsafe and zero TODO/FIXME.

# Pass 15 verified — mixed typed batches and adaptive indexed I64 Join

Pass15 replaces the I64-only typed-storage architecture with `NativeRelation::TypedColumnar` over heterogeneous `NativeColumn`s covering every current scalar carrier: Unit, Bool, I64, F64 bits, Text, live entity IDs and historical entity IDs. Physical/schema validation includes nominal entity type. Fused filter/project resolves semantic equality once and dispatches on physical column kind outside the row loop. A first naïve implementation regressed to 1.566x baseline due per-row enum dispatch and was retired; final repeated mixed-batch I64 ratios were 0.906x/0.966x/0.929x versus the same hand-written baseline.

Physical Join now has `IndexedI64IfAvailable` for direct Columnar Scan pairs. Runtime validates the pinned I64 equality contract and raw I64 key columns, builds a right-side BTree index, preserves logical Bag multiplicity/order, and falls back to semantic nested-loop for non-I64 cases. Eliminating temporary joined-row allocations reduced the indexed benchmark from an initial 1.194x baseline to final repeated ratios 1.029x/1.056x/1.104x (median 1.056x) on 20k unique-key rows. The index is still rebuilt per execution; persisted/index-maintained state remains the next step.

Final Pass15 gate: 189 declared tests, debug/release workspace tests PASS, strict Clippy/fmt/release build/rustdoc PASS, 19 crates, 18930 Rust LOC including benchmark examples, zero external Cargo sources, zero unsafe, zero TODO/FIXME and zero panic-shaped macros. See `PASS15_REPORT.md`.

---

# Pass 16 update — normative target spec and persisted index state

Pass16 adds `CFMD_IDEAL_DB_SPEC.md`, a standalone normative extraction of the final research-journal architecture updated with executable findings through this pass. It is now the preferred handoff/orientation document for future agents; the long research journal remains the derivation/history source.

Implementation additions:

- first-class `I64IndexBinding`;
- persisted `MaterializedI64IndexState` inside `PhysicalStore`;
- persisted-index reuse in adaptive I64 Join;
- explicit persisted-hit / ephemeral-build execution counters;
- Bag-correct `RelationDelta` maintenance;
- stale-index invalidation on physical relation reinstall;
- atomic `apply_relation_delta` that prepares relation and all bound materialized indexes on clones, then publishes them together;
- sequential fresh-rebuild differential tests and a post-delta physical-vs-logical vertical Join test;
- diagnostic persisted-index benchmark.

Measured result on the 20k unique-key Join diagnostic:

- ephemeral rebuild: 7.600 ms median;
- persisted reuse: 4.593 ms median;
- hand-written prebuilt BTreeMap: 3.478 ms median;
- persisted path = 0.604x old ephemeral runtime, but 1.320x prebuilt specialist runtime.

The remaining overhead is now primarily representation-level: materialized index buckets own logical `Vec<Value>` rows. The target physical contract has therefore been sharpened to typed payloads / stable row-slot handles with logical Value materialization only at semantic boundaries.

Final Pass16 verification: 193 tests; debug/release PASS; fmt PASS; strict Clippy PASS; release build PASS; strict rustdoc PASS; overflow-check plan/integration release tests PASS; 19 crates; 19,805 Rust LOC; zero external Cargo sources/unsafe/TODO/FIXME/panic-shaped macros.

---

# Pass17 implementation update

Pass17 removes copied logical payload from the persisted I64 Join index. `PhysicalStore` now owns `InstalledRelation` objects with stable `PhysicalRowId`s; `MaterializedI64IndexState` stores key -> row-handle buckets and resolves payload from the authoritative physical relation at execution time.

Standalone index-only delta maintenance was retired. `apply_relation_delta` is the single atomic maintained transition: it updates the physical relation, derives row-handle edits, updates every bound persisted index on clones, and publishes all state together only after success.

A hostile multi-delete test verifies that stable handles remain attached to the correct payload after dense physical row positions shift. Total declared tests increased from 193 to 194.

Performance on the 20k unique-key persisted self-join improved from Pass16's ~1.32x hand-written prebuilt-index gap to roughly 1.10–1.14x in final runs, while persisted reuse remains about half the runtime of rebuilding the index per query. The remaining physical issue is no longer logical-row duplication; it is typed batch/output materialization plus dense-position maintenance after deletes.

See `PASS17_REPORT.md` and the updated `CFMD_IDEAL_DB_SPEC.md` for the exact normative correction and remaining frontier.
