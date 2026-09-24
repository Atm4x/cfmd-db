# CFMD Pass50 Report — Prepared Γ-QCN / Maintained Canonical Quotient Factors

**Status:** VERIFIED on Rust 1.98.1.

**Source window:** 2026-09-20 18:03:25 → 18:19:04 +03:00 (**15m39s**). Production source was frozen at 18:19:04; no source changes were made after freeze.

**Scope:** physical planner/prepared-plan/derived-state layer only. Pass50 does not change logical query semantics, semantic authority, transaction authority, or the durable revision model.

Pass50 starts from the Pass49 Γ-Quotient Constraint Network (Γ-QCN). Pass49 established that the optimizer can factor a multiway equality graph by pinned-Γ semantic quotient coordinates and propagate finite support before original-order enumeration. Two remaining costs were architectural rather than merely micro-optimizations: the quotient basis was reconstructed on every prepared execution, and every execution recanonicalized quotient endpoints from rows even when the same revision-derived canonical factors could be maintained across relation deltas.

## 1. Problem

Pass49 still rebuilt two different layers on every admitted Γ-QCN execution:

1. **semantic quotient compilation** — flatten the eligible multiway tree, collect predicates, resolve checked refinement closure and reconstruct the quotient-coordinate basis;
2. **canonical endpoint factors** — materialize each `(relation, column, target equivalence)` canonical key from the current physical row again before quotient-domain/support propagation.

The first is stable for a prepared `(query, Γ)` plan. The second is revision-derived physical state and can use the same stable-row-handle + atomic delta machinery already used by semantic indexes.

A naïve solution would have installed those factors as ordinary persisted semantic indexes. That route was rejected because it would silently change `JoinAccessDecision`, index advisor ownership, and access-path economics merely because a prepared Γ-QCN wanted a canonical factor. A factor useful for constraint propagation is not automatically an access index the optimizer should probe as a Join strategy.

## 2. Prepared Γ-QCN program

`PreparedPlan` now optionally owns a private `PreparedSemanticQuotientProgram` compiled during `prepare_baseline` / `prepare_with_catalog` under the exact pinned semantic context.

Preparation performs the plan-shape-only part once:

```text
physical Join/FilterEqColumns tree
        ↓
leaf/column references + Γ equality predicates
        ↓
checked semantic_quotient_specs(...)
        ↓
PreparedSemanticQuotientProgram
```

The prepared program contains the quotient basis:

```text
(target equivalence, participating leaf/column coordinates)
```

It is not result data, a cache of answers, or semantic authority. The prepared executor still reads the current physical relation state and still performs Pass49's exact final Γ predicate revalidation.

`ExecutionStats.multiway_join_prepared_quotient_hits` makes prepared-basis reuse observable.

The dynamic non-prepared `Plan::execute_native` path remains correct and can still derive Γ-QCN dynamically. Prepared execution is a physical reuse optimization, not a semantic requirement.

## 3. Dedicated maintained semantic quotient factors

Pass50 adds a separate physical family inside `PhysicalStore`:

```text
semantic_quotient_factors:
    SemanticIndexBinding -> MaterializedSemanticIndexState
```

The representation intentionally reuses the verified canonical semantic-index implementation because that implementation already provides the exact properties a quotient factor needs:

- Γ-bound primitive canonical keys;
- stable `PhysicalRowId` identity;
- reverse key-by-handle lookup;
- duplicate-safe bucket membership;
- atomic relation-delta validation/application;
- semantic-context compatibility/rebuild checks.

However, the family is **separate from** `semantic_indexes`.

Consequences:

- a Γ-QCN factor does not become a normal Join access path merely by existing;
- Pass43 advisor ownership does not accidentally acquire or evict it;
- `JoinAccessDecision` economics do not change because a prepared plan requested propagation factors;
- factors remain reconstructible physical state only.

`PreparedPlan::materialize_semantic_quotient_factors(...)` explicitly materializes the missing/stale primitive factors implied by its prepared quotient basis. All candidate factors are fully built first; only after successful preparation are they published into the factor family with one physical transition-epoch advance. Repeating the operation with compatible factors is a no-op.

## 4. Atomic delta maintenance

Relation replacement continues to invalidate relation-bound derived state, including quotient factors.

For ordinary `RelationDelta`, Pass50 refactors relation-derived-state validation/application so one physical transition covers:

```text
relation
+ specialized I64 indexes
+ generic semantic indexes
+ semantic quotient factors
+ retained semantic statistics
```

Every affected derived state validates the physical delta before the installed relation or any family is mutated. Only then is the relation delta and every bound derived delta applied and one transition epoch published.

This preserves the existing law:

> derived state may accelerate execution, but relation state and pinned Γ remain authoritative.

Γ drift is never silently reinterpreted. A factor incompatible with the supplied semantic context triggers the same rebuild-required boundary as the equivalent canonical semantic index.

## 5. Execution reuse by stable handle

Pass49's quotient key cache canonicalized each required endpoint from row payloads at execution time.

Pass50 first looks for a compatible dedicated factor binding:

```text
(relation, layout, column, quotient equivalence)
```

If it exists and its row cardinality coheres with the installed relation, the key cache retrieves the exact `CanonicalEqKey` directly from the factor's reverse stable-handle map:

```text
PhysicalRowId -> CanonicalEqKey
```

No logical row materialization or recanonicalization is needed for that endpoint key.

If a compatible maintained factor is absent, execution uses the ordinary Pass49 canonicalization fallback. The optimization is therefore optional and exact.

`ExecutionStats.multiway_join_maintained_quotient_key_hits` records this reuse.

## 6. Hostile / regression evidence

Pass50 specifically verifies a three-relation exact-I64 Γ-QCN fixture with 140 physical rows across the leaves:

1. preparing the query compiles the Γ-QCN basis once;
2. explicit factor materialization creates exactly three dedicated quotient factors;
3. a second materialization is a no-op;
4. those factors are **not** visible through the ordinary semantic-index access family;
5. prepared execution equals fresh logical reference evaluation;
6. the prepared quotient program is used;
7. all 140 quotient endpoint keys are served from maintained factor state;
8. a relation delta replaces one right-side key;
9. the same atomic physical transition maintains the bound quotient factor;
10. the next prepared execution still equals fresh logical reference evaluation and again receives all 140 quotient keys from maintained factor state.

This is evidence for basis/factor reuse and delta maintenance. It is **not** evidence that the whole Γ-QCN support state is incrementally maintained.

## 7. What Pass50 deliberately does not claim

Pass50 does **not** maintain the complete Γ-QCN execution state through `Change/Dq` yet.

The following are still reconstructed per execution:

- N-way common-key domains for each quotient coordinate;
- row support masks;
- the cross-coordinate monotone support fixed point;
- final original-order tuple enumeration.

Thus the precise result is:

```text
prepared semantic basis               VERIFIED reusable
canonical endpoint factors            VERIFIED optional + delta-maintained
common domains/support/fixed point     still execution-local
```

Likewise factor materialization is explicit. Automatic create/retain/share/evict/rebuild policy, byte-level budgeting, and sharing the same factor family across unrelated prepared plans remain part of the multi-family lifecycle frontier.

## 8. Verification

Final Rust 1.98.1 gate after source freeze:

```text
cargo fmt --all -- --check                                  PASS
cargo check --workspace --all-targets                       PASS
cargo test --workspace --all-targets                        PASS
cargo clippy --workspace --all-targets -- -D warnings       PASS
cargo test --workspace --all-targets --release              PASS
cargo build --workspace --release                           PASS
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps  PASS
RUSTFLAGS='-C overflow-checks=yes' cargo test ... --release PASS
```

The first combined gate invocation reached an external timeout after fmt/check/debug tests/strict Clippy while release artifacts were compiling. Per project rules this was recorded as neither PASS nor FAIL for the unfinished stages. Release test/build/rustdoc were then run separately to completion. The first separate overflow-check compilation likewise timed out externally; the warmed repeat completed successfully. Only completed command results are reported above.

Workspace metrics at freeze:

- 357 declared Rust tests;
- 129 `kernel-plan` tests (128 normal + 1 ignored diagnostic benchmark);
- 21 crates;
- 48,758 Rust LOC under `crates/`;
- 19 pre-existing `#[allow(...)]` sites, no new suppression;
- 0 external Cargo registry/git sources;
- 0 `unsafe` in `crates/`;
- 0 TODO/FIXME/todo!/unimplemented! markers in `crates/`.

Evidence is retained under `evidence/pass50/` inside the workspace.

## 9. Problem ledger

### CLOSED exactly in Pass50 — 2

1. ✅ **Prepared Γ-QCN execution rebuilt the semantic quotient basis on every execution.** The quotient basis is now compiled once under pinned `(query, Γ)` into `PreparedPlan` and reused by prepared execution, while dynamic execution retains an exact fallback.
2. ✅ **Γ-QCN endpoint canonical keys were repeatedly reconstructed from row payloads despite being revision-derived maintainable factors.** Pass50 adds a dedicated quotient-factor family, explicit atomic materialization, stable-handle key reuse and atomic relation-delta maintenance without exposing factors as ordinary Join access indexes.

### Historical OPEN from Pass49 fully closed this pass

**0 / 22.** Pass50 closes two concrete reconstruction/maintenance defects inside the Γ-QCN implementation, but the historical multiway/lifecycle items are broader: common-domain/support/fixed-point state is still execution-local, factor lifecycle is explicit rather than advisor-driven, structural/custom canonical quotients remain fallback-only, and search remains bounded.

### Advanced but still OPEN

1. 🟨 **Γ-QCN incremental maintenance.** Basis compilation and canonical endpoint factors are reusable; common domains, support masks and the support fixed point are still rebuilt per execution. True `Change/Dq` maintenance of these dependent structures remains OPEN.
2. 🟨 **Multi-family lifecycle/advisor.** Quotient factors are now a first-class physical family, but their creation/retention/sharing/eviction/memory accounting is not yet unified with specialized I64, generic semantic indexes and retained statistics.
3. 🟨 **General multiway/bushy planning.** The verified Γ-QCN path is still the bounded 3–8-leaf builtin-primitive fragment. Adaptive/unbounded search and general indexed/typed subset execution remain OPEN.
4. 🟨 **Structural/custom equivalences.** Prepared quotient compilation can reason over checked refinement laws generally, but physical canonical quotient factors require currently supported primitive canonical keys.
5. 🟨 **I64 Group/TopK constant-factor debt.** Unchanged by Pass50.

### Historical / active OPEN after Pass50 — 22

1. ⬜ Structural/custom-equivalence canonical indexing and typed production.
2. ⬜ General nested/multiway/bushy Join planning beyond the verified bounded 3–8-leaf builtin-primitive Γ-QCN fragment: adaptive/unbounded search, richer non-equality predicate graphs and fully general indexed/typed subset execution.
3. ⬜ First-class multi-family physical access advisor/lifecycle across specialized I64, generic semantic indexes, retained statistics, Γ-QCN factors and future layouts.
4. ⬜ Autonomous workload statistics/telemetry, retention decay and lifecycle scheduling.
5. ⬜ Exact physical-index/statistics/quotient-factor byte/resident-memory budgeting and rebuild scheduling.
6. ⬜ Canonical-key encoding/version migration for long-lived physical caches/statistics/factors.
7. ⬜ Remaining physical layouts and explicit `OrderedView`/pagination.
8. ⬜ Secondary-index / alternate-layout rebuild performance after recovery.
9. ⬜ Clone-heavy runtime transaction candidates → COW/persistent roots.
10. ⬜ Durable revision DAG / branch+merge ancestry.
11. ⬜ General historical durable-format migration framework.
12. ⬜ Arbitrary/plugin semantic executable artifact packaging/signing/deployment.
13. ⬜ Transaction intent/outcome retention + GC.
14. ⬜ Streaming/chunked checkpoints/metadata.
15. ⬜ Real machine power-loss assurance and Windows/network-FS/FUSE durability semantics.
16. ⬜ General lock-poison/restart policy.
17. ⬜ Group commit / async durability.
18. ⬜ Replication / consensus and broader distribution architecture.
19. ⬜ Durable-store authentication/MAC; current CRC32C is corruption detection only.
20. ⬜ Formal power-loss proof for rename/fsync/GC protocol.
21. ⬜ Transaction repair runtime.
22. ⬜ Formal mechanization.

### New OPEN created in Pass50

**0.** Maintaining common-domain/support/fixed-point state and automating quotient-factor lifecycle are refinements of the already-existing Γ-QCN/multi-family lifecycle frontier rather than new independent subsystems.

## 10. Result / next direction

Pass50 moves Γ-QCN from a fully execution-local optimization toward a reusable CFMD-native physical program without confusing reuse with authority:

- semantic law compilation belongs to prepared `(query, Γ)` state;
- canonical quotient factors belong to reconstructible revision-derived physical state;
- exact query predicates remain the final semantic checker.

The next coherent experiment is now narrower and more meaningful than "cache more": determine whether the **quotient-domain/support fixed point itself** admits clean incremental maintenance from relation deltas without creating a second graph of bespoke invalidation rules. The preferred route is to derive that maintenance from the existing change calculus and stable row identities; if the dependency structure becomes ad-hoc, the correct result is to keep support execution-local rather than contaminate the architecture.
