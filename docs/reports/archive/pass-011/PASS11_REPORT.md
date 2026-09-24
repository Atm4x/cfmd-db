# CFMD Pass 11 — complete relational delta coverage and guarded recursive equivalence

Date: 2026-09-19
Wall-clock cycle start: 2026-09-19T10:40:56Z
Wall-clock target: 20 minutes
Wall-clock end: 2026-09-19T11:01:05Z
Measured elapsed: 20m 09s
Baseline: pass10 verified (152 tests)
Toolchain: Rust 1.98.1 standalone, local/offline
Status: **VERIFIED PASS**

## 1. `Group` no longer falls back out of the optimized relational derivative tree

Problem -> pass10 left `RelExpr::Group` as the only current relational node returning derivative fallback. A naive row-local rule is unsafe because group birth/death, semantic key equality, the empty global group, and exact aggregate state all matter.

Hypothesis -> the same safe intermediate used for Join/TopK applies: recursively obtain the child semantic delta, replay it into the old child result, and recompute only the Group operator over the reconstructed intermediate.

Implementation -> factored grouping into `group_relation_value`; added `GroupReplaySpec`, `rel_delta_group_optimized`, and `rel_delta_group_local_replay`; wired `Group` into `rel_delta_optimized_inner`.

Falsification -> three independent differential families compare the optimized result against `rel_delta_by_recompute`: 64 Count transitions with ASCII-CI group representatives and group birth/death; all 16 transitions of a two-row empty/nonempty global Count group, including the monoid identity row `[0]`; and 64 ExactF64Sum transitions with grouped finite values.

Result -> every current `RelExpr` constructor now has a compositional derivative path. Group is correctness-supported, but its implementation is still operator-local replay rather than maintained per-group aggregate state.

## 2. Guarded recursive `μ` equivalence is now an executable semantic object

Problem -> `TypeExpr::Mu/Var` already existed and value-shape validation could traverse guarded recursive values, but equality domains had no recursive representation. Structural equivalence cycles were therefore rejected indiscriminately and recursive relation columns could not have a typed semantic equality law.

Hypothesis -> recursion must be explicit rather than inferred from an arbitrary graph cycle. Structural equality therefore needs lexical `Mu` and `Var` nodes, with guardedness checked independently of runtime data. Type-variable names and semantic IDs must not leak into domain equality, so recursive domains need a canonical binder representation.

Implementation -> added `StructuralEquivalenceDef::{Mu, Var}` and `EquivalenceDomain::{Mu, Var(depth)}`. `domain_for_type` now validates guarded types and canonicalizes `TypeVar` references to de-Bruijn-like binder depth. Structural equivalence domain resolution tracks lexical semantic binders and constructor depth: free recursion yields `FreeStructuralRecursion`; a binder reached without crossing a constructor yields `UnguardedStructuralRecursion`; arbitrary non-lexical cycles remain `CyclicStructuralEquivalence`.

Structural graph validation now validates closed roots instead of incorrectly requiring recursive interior nodes to be closed standalone equivalences. The graph walk ignores `Var -> binder` as an ownership edge, so a valid recursive component has its explicit `Mu` root while rootless ordinary cycles remain rejected.

Result -> recursive type and equality domains can now match exactly without using raw binder identities.

## 3. Recursive equivalence evaluation and refinement terminate through explicit binders

Problem -> adding a recursive domain is insufficient if equality evaluation or equality refinement either loops or treats a legal recursive back-edge as the old forbidden cycle.

Implementation -> `Mu` delegates evaluation to its body and `Var` delegates to its lexical binder after the closed structural domain has been validated. Recursive refinement maintains a stack of `(finer binder, coarser binder)` pairs; matching `Var` nodes discharge against that binder relation rather than recursing indefinitely. Public refinement first validates both domains, so malformed unguarded recursion cannot be accepted coinductively by accident.

Falsification -> a finite recursive list-like value with ASCII-CI text heads compares equal across representative changes and unequal at a different recursive shape. Separate tests reject direct unguarded `Mu -> Var` and a free `Var`. A two-family recursive structure proves directional refinement `TextExact -> TextAsciiCaseInsensitive` through the recursive binder and rejects the reverse direction.

Result -> guarded recursion is supported as a finite executable equality law, while arbitrary cyclic structural definitions remain rejected.

## 4. Vertical query-level recursive-equality test

A relation column is defined with a guarded `μ` type and the corresponding recursive structural equivalence. `FilterEqConst` prepares and executes against a recursively nested literal whose text representatives differ only by ASCII case. Full `(Schema, Γ)` validation, domain matching, recursive value-shape checking, query-equivalence validation, and runtime equality all participate in the same test.

This closes the previous gap where recursive equality could have existed only as an isolated registry feature.

## 5. Final verification gate

Final source passes:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo test --workspace --release`
- `cargo build --workspace --release`

Final audit:

- 159 `#[test]` declarations (152 -> 159);
- 18 workspace crates;
- 14,466 Rust LOC;
- no external Cargo lockfile sources;
- zero `unsafe` in Rust sources;
- zero TODO/FIXME;
- zero `panic!`, `todo!`, `unimplemented!` macros;
- one test-only `#[allow(clippy::too_many_lines)]` on the large vertical recursive fixture; production code has no Clippy suppression.

## 6. Remaining problems, ordered by importance

1. **Incremental state is still reconstructed instead of persisted.** Set projection/Distinct support counts are recomputed from the old intermediate; Group/Join/TopK use safe local replay. The next architectural boundary should make certified materialized operator state explicit rather than hiding scans behind derivative helpers.
2. **Join is not yet a true indexed `ΔJoin`.** Correctness is covered, including two-sided simultaneous deltas, but asymptotic work still scales with replayed intermediate relations.
3. **TopK is not maintained with order statistics.** Threshold/tie correctness is exact, but updates replay sorting at the operator node.
4. **Group lacks maintained aggregate state.** `ExactCount` can in principle update by group deltas; `ExactF64Sum` requires retaining its exact accumulator rather than only the rounded visible result.
5. **OrderedView/pagination is absent.** `TopKWithTies` deliberately returns ordinary Set/Bag semantics; stable logical order and cursor semantics need their own type/contract.
6. **Physical lowering / PlanIR and abstraction-tax benchmarks remain the main system-level blocker.** The semantic kernel is now substantially more complete than the evidence that it can lower to specialist physical layouts without mandatory overhead.
7. **Durability/engine work is still outside the verified kernel:** WAL/recovery, persistent indexes/materializations, compaction, concurrency/distribution and crash testing remain future implementation layers.

## Bottom line

Pass11 closes both immediate correctness holes named by pass10: `Group` derivative fallback and guarded recursive `μ` equivalence. The current relational IR now has a correctness-supported derivative path for every constructor, and recursive algebraic values can carry explicit, typed, guarded semantic equality through registry, schema validation and query execution. The frontier has moved from missing logical operators toward maintained incremental state and physical lowering/performance.
