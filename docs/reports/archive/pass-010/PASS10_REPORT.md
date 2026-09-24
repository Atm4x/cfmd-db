# CFMD Pass 10 — support-count Set deltas and compositional local replay

Date: 2026-09-19
Wall-clock cycle start: 2026-09-19T10:04:28Z
Wall-clock target: 20 minutes
Wall-clock end: 2026-09-19T10:24:29Z
Measured elapsed: 20m 01s
Baseline: pass09 verified (145 tests)
Toolchain: Rust 1.98.1 standalone, local/offline
Status: **VERIFIED PASS**

## 1. Set projection no longer requires correctness fallback

Problem -> pass09 deliberately returned `None` for `ΔProject(Set)`. Removing one source row can leave the projected Set unchanged when another source row still supports the same projected equivalence class.

Hypothesis -> Set projection is incrementally decidable if the old input supplies semantic support counts for each projected equivalence class. Output membership changes only on support transitions `0 -> positive` and `positive -> 0`.

Implementation -> added semantic support accounting over rows, using the result relation's pinned column equivalences rather than Rust equality. `ΔProject(Set)` evaluates the old child once, projects it into support classes, applies projected child removals/insertions to those counts, and emits only membership-boundary crossings. Inconsistent negative/absent support transitions are rejected as `InconsistentIncrementalDelta`.

Falsification -> the old hostile case `(A,1),(A,2) -> (A,2)` projected to `A` now returns a supported empty delta instead of fallback. An exhaustive 64-transition state-space test over three Set rows, including two rows collapsing to one projection, agrees with full recomputation for every old/new state pair.

Result -> the correctness blocker for generic Set projection is closed. Performance is not yet fully incremental because support state is reconstructed from the old child result rather than persisted/materialized.

## 2. Distinct and PromoteToBag gained compositional deltas

Problem -> both operators still forced recomputation fallback despite having sufficient child-delta information once semantic support counts exist.

Hypothesis -> `Distinct` is the same support-boundary problem as Set projection, but without a projection map. `PromoteToBag` is a representation-level semantic change whose inserted/removed row occurrences are inherited from the child delta.

Implementation -> `Distinct` now builds old semantic support counts under its output equivalences, applies child deltas, and emits only class appearance/disappearance. `PromoteToBag` transports inserted/removed rows while replacing only the result `RelType` semantics with Bag.

Falsification -> duplicate-representative removal under ASCII-CI equality leaves Distinct unchanged. A 64-transition hostile state-space test for `Project(Bag) -> Distinct` agrees with the recomputation oracle. PromoteToBag has an explicit oracle-equivalence regression test.

Result -> `Distinct` and `PromoteToBag` are no longer unsupported delta nodes.

## 3. Join gained operator-local replay

Problem -> a sound true delta join needs indexed/provenance state and careful multiplicity accounting; implementing an ad-hoc fast rule would create another correctness surface.

Hypothesis -> an intermediate safe step is compositional local replay: derive both child deltas recursively, reconstruct the new child relation values from old values plus those semantic deltas, then recompute only the Join node.

Implementation -> added semantic `apply_relation_delta_to_value`, including relation-kind and Set-duplicate consistency checks; factored Join evaluation over `RelationValue`; added `relation_delta_between_values`; and wired `JoinEq` into the optimized derivative tree using child deltas.

Falsification -> a two-sided exhaustive test covers all 256 transitions between four-bit left/right states with ASCII-CI join equality, including simultaneous updates to both inputs. Every local-replay delta is semantically equivalent to the full recomputation oracle.

Result -> Join is compositionally supported without evaluating the changed child trees from scratch. This is a correctness/architecture improvement, not yet an asymptotically fast indexed `ΔJoin`.

## 4. TopKWithTies gained operator-local replay

Problem -> threshold changes and ties make a naive row-local TopK delta unsound.

Hypothesis -> reconstruct the new child relation from its delta, then replay only the deterministic TopK-with-ties operator under the already-certified ordering/equality law.

Implementation -> factored TopK evaluation over a `RelationValue` and added a local-replay derivative node. Child recomputation on the new model is avoided; ordering and exact tie semantics remain unchanged.

Falsification -> an exhaustive 64-transition Bag state space over `[1,2,2]` with `k=1` exercises threshold disappearance/appearance and tied multiplicity. Every delta agrees semantically with the recomputation oracle.

Result -> TopK is now compositionally supported. As with Join, this is local operator replay, not an order-statistics incremental algorithm.

## 5. Cross-operator composition falsifier

A composed `Project -> Distinct` on both sides, then `Join -> PromoteToBag`, was tested over all 256 old/new two-relation states. The recursively optimized delta tree agrees with full recomputation in every transition. This specifically checks that individually-correct delta nodes remain correct when their semantic Set/Bag boundaries are nested.

## 6. Final verification gate

The final source passes:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo test --workspace --release`
- `cargo build --workspace --release`

Final audit:

- 152 `#[test]` declarations;
- 18 workspace crates;
- 13,675 Rust LOC;
- no external Cargo lockfile sources;
- zero `unsafe` in Rust sources;
- zero TODO/FIXME;
- zero `panic!`, `todo!`, `unimplemented!` macros.

## 7. Remaining problems, ordered by importance

1. **Guarded recursive `μ` equivalence remains the main semantic hole.** Recursive types validate and execute structurally, but `domain_for_type` has no `Mu/Var` domain representation and structural-equivalence cycle detection currently rejects the graph shape a recursive equivalence would need. The next fix must introduce an explicitly guarded finite recursive-domain/equivalence representation; merely allowing cycles would risk nontermination and would not solve domain matching.
2. **`Group` is the only current `RelExpr` node still returning delta fallback.** The safe next step is to factor grouping over an intermediate `RelationValue` and add local replay, including the empty-input/global-group case. The real IVM step after that is maintained per-group `ExactCount` / `ExactF64Sum` state with group birth/death handling.
3. **Set support state is reconstructed, not persisted.** `ΔProject(Set)` and `Distinct` are correct but still scan the old child result to rebuild supports. A materialized/certified support-count or provenance state is needed for genuine low-cost IVM.
4. **Join and TopK are local replay, not true asymptotically-fast deltas.** Join needs indexed delta joins/multiplicity accounting; TopK needs maintained order-statistics/threshold state.
5. **OrderedView/pagination is still absent.** `TopKWithTies` intentionally returns ordinary Set/Bag semantics rather than smuggling row order into relations.
6. **Physical lowering/PlanIR and abstraction-tax benchmarks remain system-level blockers.** Semantic correctness is substantially ahead of the physical engine/compiler evidence.
7. **Durable DB concerns remain outside this prototype checkpoint:** WAL/recovery, persistent indexes/materializations, compaction, concurrency/distribution and crash testing are not yet the subject of the verified kernel.

## Bottom line

Pass10 removes the Set-projection fallback, extends the oracle-checked delta tree through Distinct, PromoteToBag, Join and TopK, and validates recursive composition over hostile finite state spaces. The correctness frontier inside the current relational IR is now concentrated in `Group`; the deeper semantic frontier is guarded recursive `μ` equivalence. The major remaining risk has shifted further toward maintained incremental state and physical lowering/performance rather than basic delta semantics.
