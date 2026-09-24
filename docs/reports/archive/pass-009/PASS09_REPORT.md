# CFMD Pass 09 — executable restoration, representative-safe equality, incremental relational deltas, checked ordering compatibility

Date: 2026-09-19
Wall-clock cycle start: 2026-09-19T09:42:19Z
Baseline: pass08 source checkpoint (136 declared tests, previously uncompiled)
Toolchain: Rust 1.98.1 standalone, local/offline
Final declared tests: 145
Final Rust LOC: 12,820
Status: **VERIFIED PASS**

## 1. Pass08 executable gate restored and concrete compiler failures closed

Problem -> pass08 had only source-level confidence because the previous container lacked a working Rust toolchain. Once Rust 1.98.1 was restored, the exact source exposed one real compile failure plus fmt/clippy failures.

Hypothesis -> fix only concrete gate failures first, before extending semantics.

Implementation -> replaced the ambiguous empty `vec![]` assertion with `is_empty()`, normalized formatting with rustfmt, and rewrote the ordering/equality pattern to satisfy strict pedantic Clippy without suppressions.

Falsification -> reran the complete workspace rather than only `kernel-query`.

Result -> the original pass08 functionality is now executable and verified. The restored baseline passed debug tests, release tests, release build, rustfmt and strict Clippy before further pass09 work began.

## 2. Hostile representative leak found in equality-based query operators

Problem -> pass08 correctly required ordering to be congruent with relation equality, but equality-based operators still checked only scalar/domain compatibility. Under relation equality `TextAsciiCaseInsensitive`, replacing stored representative `"A"` by `"a"` is semantically invisible. A filter/join/distinct/group using finer `TextExact` equality could nevertheless observe that replacement. This contradicts semantic Bag/Set equality and invalidates incremental deltas that legitimately cancel representative substitutions.

Hypothesis -> an equality used by an exact query must be no finer than the equality already defining the input relation column. Formally, if input equality is E and query equality is F, require `x E y => x F y` (E refines F). By equivalence transitivity this makes query predicates/grouping invariant under changes of E-representative.

Implementation -> added `SemanticRegistry::equivalence_refines`. Leaf law currently includes identity of contracts plus `TextExact -> TextAsciiCaseInsensitive`; structural equivalences recurse compositionally through Product, Sum, Option, Set, Bag, Seq and Map. `FilterEqConst`, `JoinEq`, `Distinct` and grouped keys now reject `EquivalenceNotCongruentWithInputEquality` when this law fails.

Falsification -> hostile tests verify:
- ASCII-CI relation + TextExact filter is rejected before execution;
- TextExact relation + ASCII-CI filter is accepted (correct direction);
- join/distinct/group cannot refine ASCII-CI input equality to TextExact;
- structural Product<Option<Text>> refinement is directional and compositional.

Result -> equality-based exact queries can no longer leak stored representatives that the relation semantics declared equivalent.

## 3. Optimized relational derivative path added against the recomputation oracle

Problem -> pass08 defined `RelationDelta` only by full recomputation. The next frontier explicitly required optimized `ΔScan`, then `ΔFilter`, then `ΔProject` while retaining recomputation as the correctness oracle.

Hypothesis -> build a conservative compositional fast path that returns `None` whenever local delta information is insufficient, rather than guessing.

Implementation -> added `rel_delta_optimized`:
- `ΔScan` reads only the changed base relation and computes semantic inserted/removed rows directly;
- `ΔFilter` filters inserted/removed occurrences using a representative-safe equality law;
- `ΔProject` is implemented for Bag inputs and cancels projected insert/remove pairs semantically;
- unsupported operators return `None` for fallback to `rel_delta_by_recompute`;
- Set projection explicitly returns `None`, because projection can collapse distinct source rows and an input delta alone does not carry enough support-count information.

Falsification -> a Set hostile case has old rows `(A,1),(A,2)`, removes `(A,1)`, and projects only `A`. The exact output is unchanged although naïvely projecting the removal would claim a deletion. The optimizer therefore demonstrably must fall back. A 64 old/new-state exhaustive Bag test checks the supported `Scan -> Filter -> Project` path against the oracle.

Result -> the first actual incremental relational derivative pipeline exists with an explicit soundness boundary instead of an all-or-nothing optimizer.

## 4. RelationDelta correctness is semantic, not Rust structural equality

Problem -> the exhaustive test produced a hostile counterexample where fast delta inserted `"A"` and recomputation inserted `"a"`. Under the result's ASCII-CI equality these deltas are the same semantic multiset, but Rust `PartialEq` says they differ.

Hypothesis -> fast/oracle verification must compare deltas under the `RelType` equivalences, exactly as relational results already do.

Implementation -> added `relation_deltas_semantically_equivalent`, comparing inserted and removed sides as semantic multisets under the delta's full result type.

Falsification -> the state-space test contains the concrete `"A"`/`"a"` representative mismatch and now accepts it only through semantic equality, while still rejecting multiplicity/content differences.

Result -> delta correctness no longer reintroduces physical representative bytes through the test/proof boundary.

## 5. Ordering compatibility moved behind a checked-certificate admission boundary

Problem -> pass08 runtime compatibility used a direct hardcoded `matches!` table even though semantic implementations already used checked certificates. The report explicitly listed this mismatch as the next frontier.

Hypothesis -> runtime query preparation should consume admitted compatibility facts, while a dedicated checker decides whether an ordering/equality law certificate is valid.

Implementation -> added:
- `OrderingCompatibilitySpec`;
- `OrderingCompatibilityArtifact`;
- `OrderingCompatibilityChecker`;
- `certify_ordering_compatibility`;
- registry storage/admission of checked compatibility specs.

Builtin laws are bootstrapped through the checker. `ordering_congruent_with_equivalence` now resolves the pinned implementation contracts and asks the registry for an admitted compatibility fact; it no longer contains the compatibility `matches!` table itself.

Falsification -> a certificate for `TextAsciiCaseInsensitive × TextAsciiCaseInsensitive` is accepted; `TextBinary × TextAsciiCaseInsensitive` is rejected with `OrderingCompatibilityViolation`. Existing TopK hostile tests remain green.

Result -> ordering compatibility now follows the same proof-token architectural boundary as other trusted semantic facts.

## 6. Final verification gate

All of the following pass on the final source:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo test --workspace --release`
- `cargo build --workspace --release`

Final audit:

- 145 `#[test]` declarations;
- 18 workspace crates;
- 12,820 Rust LOC;
- zero external Cargo lockfile sources;
- zero `unsafe` occurrences in Rust sources;
- zero TODO/FIXME occurrences;
- zero `panic!`, `todo!`, or `unimplemented!` macros in Rust sources.

## 7. Remaining frontier

1. Guarded recursive `μ` equivalence remains open.
2. Set-project incremental maintenance needs support counts/provenance (or an equivalent certified state) before generic `ΔProject(Set)` can avoid fallback.
3. Optimized deltas for Distinct/Join/Group/TopK/PromoteToBag remain open; recomputation remains the correctness fallback.
4. A real `OrderedView`/pagination semantic type is still intentionally deferred until the above semantic/incremental boundaries stabilize.
5. Physical lowering/PlanIR breadth and abstraction-tax benchmarks remain the larger system-level frontier from the research report.

## Bottom line

Pass08 is no longer source-only. Pass09 turns it into a fully compiled/verified checkpoint, closes a previously unnoticed representative-observation hole across equality operators, establishes the first oracle-checked incremental relational pipeline, fixes semantic delta comparison, and closes the ordering-compatibility proof-token item. The next semantic blocker is now recursive `μ` equivalence; the next incremental blocker is Set projection support accounting rather than basic delta semantics.
