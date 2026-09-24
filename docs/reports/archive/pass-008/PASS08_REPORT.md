# CFMD Pass 08 — ordering congruence, exact TopK-with-ties, semantic relational delta oracle

Date: 2026-09-19
Wall-clock target: 15 minutes
Baseline: pass07 (131 verified tests)
Pass08 source test declarations: 136
Verification status: **SOURCE-ONLY / NOT COMPILED IN THIS CONTAINER**. The restored container lost the previously installed Rust toolchain. The old installer/tar were removed to conserve storage before that was noticed; external binary download is blocked. No claim of PASS is made for pass08 until the same source is compiled again.

## 1. Ordering became observable without leaking physical row order

Problem -> pass07 had versioned ordering modules, but no exact relational operator used them. A naive `LIMIT k` would be nondeterministic under ties and would tempt downstream code to treat physical row order as semantics.

Hypothesis -> the first exact operator should return an ordinary Set/Bag subset and make tie behavior explicit, rather than introducing an implicit ordered relation.

Implementation -> added `RelExpr::TopKWithTies { input, column, ordering, direction, k }`. It returns the same Set/Bag semantic class as its input. Runtime uses a fallible deterministic comparison path; comparator failures remain typed errors. `k=0` is exact empty output. The operator is transported recursively through AtomId isomorphisms.

Falsification -> for Bag<I64> values `[2,3,1,2]`, ascending k=2 selects `{1,2,2}` and descending k=2 selects `{3,2,2}`: a threshold tie is never broken by incidental row position.

Result -> ordering is now query-observable without claiming that the resulting Bag/Set is itself ordered. A future `OrderedView` remains a distinct semantic type.

## 2. Matching scalar type is insufficient: ordering must be congruent with equality

Problem -> a Text ordering can be type-correct but still be ill-defined on the relation's quotient semantics. Under ASCII-CI equality, `"A" ≡ "a"`; binary ordering distinguishes those representatives, so TopK could change when only representative bytes changed.

Hypothesis -> exact ordering over a relation column needs a compatibility law, not merely the same scalar domain:

`x ≡_E y => compare_O(x,z) = compare_O(y,z)` and symmetrically in the other argument.

Implementation -> added `SemanticRegistry::ordering_congruent_with_equivalence`. Current admitted pairs include I64Ascending/I64Exact, F64Total/F64Bitwise, TextBinary/TextExact, TextAsciiCaseInsensitive/TextExact, TextAsciiCaseInsensitive/TextAsciiCaseInsensitive, and TextAsciiCaseInsensitiveThenBinary/TextExact. Added `RelQueryError::OrderingNotCongruentWithEquality`; TopK checks this during prepare before reading data.

Falsification -> TextBinary over a column with ASCII-CI relation equality is rejected before execution. A new `TextAsciiCaseInsensitive` preorder is accepted and causes `TopKWithTies(k=1)` to retain both `"A"` and `"a"` when they occupy the threshold equivalence class.

Result -> exact query semantics no longer depend on which representative of an equality class happens to be stored.

## 3. F64 ordering is explicit and total

Problem -> host/IEEE partial comparison is not a valid exact ordering primitive because NaN is unordered; the model also already treats signed zero and NaN payloads as visible under F64Bitwise equality.

Implementation -> added `OrderingModule::F64Total` and `OrderingDomain::F64`, using Rust/IEEE total-order behavior (`total_cmp`) over exact stored bit patterns. It is admitted as congruent with `F64Bitwise` equality.

Falsification -> source tests assert `-0.0 < +0.0` and `+infinity < positive NaN` under F64Total.

Result -> future TopK/order operations do not need hidden host floating semantics.

## 4. Fine relational deltas now have a semantic oracle

Problem -> pass07 relational derivative correctness still returned `Change::Replace(full_result)` when changed. Before implementing fast `ΔScan/ΔFilter/ΔProject`, the system needs an exact definition of the output delta under Set/Bag equivalence and multiplicity.

Implementation -> added `RelationDelta { inserted, removed, result_type }` plus `rel_delta_by_recompute`. The delta is bound to the full `RelType` so it cannot be detached from Set/Bag/equality semantics. Matching is one-to-one over semantic row equivalence, so Bag multiplicity is respected and representative substitutions inside one equivalence class cancel to an empty delta.

Falsification -> `"A" -> "a"` under ASCII-CI Bag semantics yields no inserted/removed rows. Adding a second equivalent occurrence yields exactly one inserted occurrence.

Result -> this is deliberately still recomputation-cost. It is the correctness oracle for future optimized relational derivatives; performance rules can fail/fallback without affecting semantics.

## 5. Static audit performed despite missing compiler

- all edited Rust files have balanced `{}/[]/()` under a lightweight source lexer;
- every new `RelExpr::TopKWithTies` recursive transport site currently known is updated;
- new OrderingModule variants are covered by domain/digest/compare paths;
- source tree contains 136 `#[test]` declarations (baseline verified 131 + 5 pass08 tests);
- no external dependency was added.

This is not a substitute for `cargo fmt`, `clippy` or compilation. Pass08 remains explicitly unverified until Rust is restored.

## 6. Next executable frontier after compiler restoration

1. Compile/fmt/clippy this exact checkpoint first; fix only concrete errors before adding features.
2. Use `RelationDelta` as oracle for optimized `ΔScan`, then `ΔFilter`, then `ΔProject`.
3. Give ordering compatibility the same proof-token admission path as semantic implementations instead of a builtin compatibility table.
4. Implement guarded recursive `μ` equivalence.
5. Only after those pass, introduce a real `OrderedView`/pagination semantic type.
