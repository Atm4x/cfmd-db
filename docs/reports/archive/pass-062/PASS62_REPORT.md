# PASS62 REPORT — algebraic native structural layout integration

Status: **VERIFIED**

Production source window: **2026-09-20 20:58:31 UTC → 21:18:36 UTC** (20m05s). Production source was frozen at the end of that window. All subsequent work was verification, benchmarks, documentation and packaging; no `crates/` source changed after freeze.

## Problem

The independent Program4 R&D branch showed that CFMD's full structural value algebra could be represented as recursive native physical columns instead of repeatedly materializing `Value` trees. However the closeout patch was based on Pass60, while authoritative Pass61 already contained Program3 dense revision-local identity and structural Γ-QCN factor changes. Blind patch application would therefore risk reintroducing stale physical assumptions.

The R&D patch also had two architectural weaknesses that required hostile review before promotion:

1. structural `FilterEqConst` consumption was introduced as a pattern-specific fused path rather than as a first-class predicate in the existing compositional typed-batch compiler;
2. the public `AlgebraicNativeColumn::from_values` constructor trusted arbitrary recursive `TypeExpr` values and could recursively resolve an invalid non-empty `μX.X` indefinitely instead of rejecting the unguarded type at admission.

## Hypothesis

Program4 is production-worthy only if the structural layout remains a reconstructible physical derivative and composes with the existing Pass61 execution/authority boundaries:

```text
validated TypeExpr + Value + pinned Γ
        -> AlgebraicNativeColumn
        -> typed-batch / stateful physical consumers
        -> exact logical result
```

No physical tag, offset, local ID or canonical key may become semantic authority. Structural canonicalization must remain exactly the same law as `SemanticRegistry::canonical_equivalence_key`.

## Implementation

### 1. Recursive algebraic native column family

`kernel-plan` now contains `AlgebraicNativeColumn` and `NativeColumn::Algebraic` for the current structural algebra:

- Product -> child native columns per semantic field;
- Sum -> row tags + payload ordinals + per-tag payload columns;
- Option -> optional payload ordinal + dense payload;
- Seq / Set -> offsets + flattened payload;
- Bag -> offsets + flattened payload + multiplicities;
- Map -> offsets + flattened key/value payloads;
- guarded `Mu/Var` -> recursive typed child representation;
- scalar leaves -> existing native scalar `NativeColumn` families.

`NativeRelation::typed_from_rows` keeps scalar columns specialized and lowers structural columns into this family. Row materialization remains an output/semantic-boundary operation, not mandatory internal storage.

### 2. Structural predicates are compositional typed-batch predicates

The R&D branch's direct structural filter was generalized into `TypedBatchPredicateKind`:

```text
Primitive(BoundPrimitivePredicate)
Algebraic { equivalence, expected: CanonicalEqKey }
```

`build_typed_batch_program` now admits schema-declared structural equality when the physical predicate column is algebraic. The expected key is computed once through pinned Γ; candidate rows obtain their canonical key directly from algebraic storage.

This means structural `FilterEqConst` is not limited to one `Filter -> Project` specialization. The same selection can feed existing typed stateful nodes. A hostile `Filter(structural Product<TextAsciiCI>) -> Group Count` test observes both `typed_batch_chain_hits` and `typed_stateful_batch_hits` without full input-row materialization.

The exact-I64 microkernel remains isolated: it accepts only `Primitive(I64(...))` predicates, so structural generalization does not silently enter the specialist I64 loop.

### 3. Pass60 Program4 rebased against Pass61 Program3

The integration preserves `NativeColumn::DenseLiveEntityIds` as a sibling specialized scalar family. A hostile fixture combines:

- an algebraic structural predicate column;
- a dense revision-local `LiveEntityRef` projection column;
- authoritative `PhysicalStore::apply_relation_delta` with mixed remove+insert.

After physical compaction/mutation, structural filtering still preserves logical scan order and dense local IDs reconstruct the exact external `EntityId` values.

Nested `LiveRef` leaves *inside* an algebraic structural column currently use the ordinary external-ID leaf representation. Automatically selecting dense-local nested leaves is a future multi-family lowering/advisor problem, not a semantic correctness defect.

### 4. Mutation authority surface narrowed

R&D exposed low-level algebraic selection/mutation/canonical-key helpers publicly. In production these are crate-internal. External callers may construct/read the reconstructible layout, but relation mutation must continue through the existing `PhysicalStore` atomic candidate/stable-handle transition boundary rather than bypassing it through column-local edits.

### 5. Guarded recursion is enforced at the public layout boundary

`AlgebraicNativeColumn::from_values` now calls `TypeExpr::validate()` before recursive compilation. A hostile non-empty `μX.X` is rejected with `PhysicalTypeMismatch` instead of recursively expanding `Var -> Mu -> Var`.

Already-validated internal single-value mutation checks use an internal constructor and do not repeatedly rerun the complete type validation.

### 6. Canonical-law differential hostile test

One composed structural row containing `Option + Seq + Set + Bag + Map + Sum` is canonicalized two ways:

1. directly from `AlgebraicNativeColumn`;
2. by authoritative `SemanticRegistry::canonical_equivalence_key` over the logical `Value`.

The keys are required to be exactly equal row-by-row. Case-different rows also collapse to the same expected structural class under `TextAsciiCaseInsensitive`.

## R&D hostile review result

Program4 is accepted as a production integration candidate, but not mechanically merged. The authoritative integration:

- preserves Pass61 dense-reference and Γ-QCN changes;
- promotes the representation family;
- replaces the pattern-specific structural-filter boundary with the general typed-batch predicate path;
- narrows mutation APIs;
- fixes the unguarded-recursion admission defect;
- retains exact logical/Γ authority.

The R&D package's incomplete overflow gate is superseded by the authoritative Pass62 full gate, which passes.

## Performance evidence

Frozen-code, release, five separate process-level runs. These are representation diagnostics only.

### Product field scan

Logical fixture: `Value::Product(BTreeMap<...>)`; native fixture: direct child I64 array.

Ratios: **25.634x, 50.328x, 40.805x, 41.834x, 46.305x**.  
Median: **41.834x**; range **25.634–50.328x**.

### Sum tag scan

Logical fixture: boxed `Value::Variant`; native fixture: dense semantic-tag vector.

Ratios: **1.732x, 2.016x, 1.806x, 1.843x, 1.508x**.  
Median: **1.806x**; range **1.508–2.016x**.

The process spread is substantial, so neither diagnostic is a general query-engine speed claim. The useful result is narrower: the structural representation does not require a mandatory logical `Value` wrapper on these hot access patterns.

## Result

### CLOSED exactly in Pass62 — 3

1. **The full current structural type algebra had no integrated recursive native physical column family.** Product/Sum/Option/Seq/Set/Bag/Map/guarded Mu/Var now lower to exact reconstructible algebraic columns inside ordinary `TypedColumnar` storage.
2. **Structural Γ equality consumption was pattern-specific rather than compositional.** Structural `FilterEqConst` is now a normal typed-batch predicate and can feed downstream typed stateful operators without mandatory logical-row materialization.
3. **Public algebraic layout construction did not reject invalid unguarded recursive types before descent.** The public constructor now enforces `TypeExpr::validate()`; hostile non-empty `μX.X` is rejected safely.

These are production problems closed in this pass; they do **not** mean the broad historical “remaining physical layouts” or “structural/custom semantic indexing” items are fully closed.

### Historical OPEN accounting — audit correction

Pass61 reported **22** historical architectural OPEN items. That numbered list had, since Pass43, omitted two explicitly still-open performance debts that continued to appear in the reports/specification:

- maintained I64 Group constant-factor gap;
- maintained I64 TopK constant-factor gap.

Pass62 therefore normalizes the authoritative ledger to **24 active OPEN**. This is a bookkeeping correction, not two newly discovered problems.

- Historical architectural OPEN fully closed this pass: **0 / 22**.
- Restored previously omitted performance OPEN: **2**.
- Active OPEN after Pass62: **24**.
- Genuinely new OPEN created: **0**.

### Active OPEN after Pass62 — 24

1. ⬜ Structural/custom-equivalence physical indexing is still incomplete: durable recursive-key encoding/versioning, persisted structural indexes, arbitrary/plugin canonical laws and structural ordering remain open.
2. ⬜ General nested/multiway/bushy Join planning beyond the verified bounded 3–8-leaf Γ-QCN fragment: adaptive/unbounded search, richer predicate graphs and fully general indexed/typed subset execution.
3. ⬜ First-class multi-family physical advisor/lifecycle across specialized I64, generic semantic indexes, retained statistics, Γ-QCN factors/support, algebraic layouts and future layouts.
4. ⬜ Autonomous workload statistics/telemetry, histograms/correlation, decay/hysteresis and lifecycle scheduling.
5. ⬜ Exact byte/resident-memory accounting, shared-layout budgeting and rebuild scheduling for indexes/statistics/QCN/layout families.
6. ⬜ Canonical-key/cache encoding-version migration and rebuild/compatibility law for long-lived physical artifacts.
7. ⬜ Remaining physical layouts beyond the new algebraic family, plus explicit `OrderedView`/pagination.
8. ⬜ Secondary-index / alternate-layout rebuild economics and fast reconstruction after recovery.
9. ⬜ Persistent outer artifact-map metadata: Pass57 removed payload-size clone tax, but catalogs remain ordinary `BTreeMap<K, Arc<State>>` with O(number of artifacts) candidate clone metadata.
10. ⬜ Durable revision DAG / branch+merge ancestry and merge replay.
11. ⬜ General historical durable-format migration framework.
12. ⬜ Arbitrary/plugin semantic executable artifact packaging/signing/authentication/deployment.
13. ⬜ Transaction intent/outcome retention + GC.
14. ⬜ Streaming/chunked checkpoints and metadata.
15. ⬜ Real machine power-loss assurance plus Windows/network-FS/FUSE durability semantics.
16. ⬜ General lock-poison/restart policy.
17. ⬜ Group commit / async durability.
18. ⬜ Replication / consensus and broader distribution architecture.
19. ⬜ Durable-store authentication/MAC; CRC32C remains corruption detection only.
20. ⬜ Formal power-loss proof for rename/fsync/GC protocol.
21. ⬜ Transaction repair runtime.
22. ⬜ Formal mechanization of the remaining semantic/transport/retention/power-loss obligations.
23. ⬜ Maintained I64 Group constant-factor gap (restored ledger item; still performance debt, not a correctness gap).
24. ⬜ Maintained I64 TopK constant-factor gap (restored ledger item; still performance debt, not a correctness gap).

### Advanced but still OPEN

- historical #1 is substantially narrower: structural in-memory representation, structural Group/Join, relation quotienting, Γ-QCN factors and compositional structural filtering are production; persistence/plugin/ordering remain;
- historical #7 is narrower because algebraic structural columns are now one real physical layout family, but KeyValue/Adjacency/CSR/DenseArray/Inverted/Custom and OrderedView remain;
- multi-family lifecycle must eventually decide not only whether to create an index, but when an algebraic/chunked/dense-local representation is worth retaining under a real byte budget;
- nested algebraic LiveRef leaves do not automatically choose Program3's dense-local identity representation yet.

## Verification

Rust: `rustc 1.98.1 (48a229cea 2026-09-01)`.

Final frozen bytes PASS:

- `cargo fmt --all -- --check`
- `cargo check --workspace --all-targets`
- `cargo test --workspace --all-targets`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace --all-targets --release`
- `cargo build --workspace --release`
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps`
- `RUSTFLAGS='-C overflow-checks=yes' cargo test --workspace --all-targets --release`

The first cold combined release command timed out while compiling and was not counted. The first overflow-check release invocation likewise hit the external compilation timeout. Warmed standalone retries completed successfully; no timeout is represented as PASS or FAIL.

Static snapshot before packaging:

- **401 declared tests**;
- **151 `kernel-plan` tests**;
- **21 crates**;
- **55,476 Rust LOC**;
- **0 `unsafe` hits**;
- **19 existing `#[allow(...)]`**, no new suppression;
- **0 TODO/FIXME/todo!/unimplemented! hits**;
- **0 `GenericScan` hits**;
- **0 external registry/git Cargo sources**.

## Next frontier

Pass62 closes the in-memory structural *representation* gap but makes the remaining system problem clearer: CFMD now has several useful physical families whose ownership/admission policy is fragmented. The highest architectural payoff for Pass63 is a **multi-family physical lifecycle + exact memory-accounting pass** that can reason about semantic indexes, statistics, Γ-QCN factors/support and algebraic layouts under one reconstructible benefit/budget contract.

A parallel lower-level alternative remains durable structural canonical-key format/migration. Do not add another isolated structural consumer unless hostile evidence shows a concrete missing execution path.
