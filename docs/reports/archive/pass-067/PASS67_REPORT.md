# PASS67 REPORT — semantic-statistics lifecycle + persistent physical catalog roots

Status: **VERIFIED after frozen-source Rust and packaging gates**.

Authoritative base: verified Pass66. Corrected Program7 was still external R&D and was not used by this pass.
Production source window: **2026-09-20 23:58:08 UTC → 2026-09-21 00:18:20 UTC (20m12s)**.
Production source freeze: **2026-09-21 00:18:20 UTC**. `crates/` SHA-256 at freeze is byte-identical to the post-gate snapshot.

## Problem A — semantic statistics remained manual-only

Pass46 introduced exact reconstructible Γ-bound key-cardinality statistics, but through Pass66 they had no autonomous create/retain/evict lifecycle. This left an avoidable transient-index tax on duplicate-heavy direct joins: without exact retained cardinality, the conservative planner may build a transient index because it must assume more distinct keys than actually exist.

## Implementation A — conservative semantic-statistics advisor

Pass67 extends the existing physical-artifact advisor to `MaterializedSemanticStatisticsState` without treating statistics as semantic authority.

Production advice is deliberately narrow and falsifiable:

- workload admission is limited to direct primitive `JoinEq` shapes whose executor/cost model actually consumes exact key-cardinality information;
- a candidate statistic is profitable only when its exact distinct count changes the existing decision from transient-index construction to the cheaper scan and the saved repeated execution work amortizes statistics construction;
- compatible persisted I64 or semantic indexes suppress duplicate statistics advice because they already provide exact join cardinality/access;
- managed/global retained-byte budgets are enforced through the Pass63 memory discipline;
- only advisor-owned statistics are evictable;
- explicit `install_semantic_statistics()` transfers an advisor-owned artifact to manual ownership/pin;
- runtime publication advances once only when physical state actually changes; an identical second advice turn is a no-op publication.

This is not a claim that all statistics now have one autonomous lifecycle. Multiway statistics can change join-order search itself and therefore require a counterfactual planner score; histograms/correlation/read-write telemetry/decay remain OPEN.

## Hostile evidence A

The production hostile `semantic_statistics_advisor_prevents_duplicate_heavy_transient_build_and_evicts_owned` executes the same duplicate-heavy direct Join before and after advice:

- before retained statistics: `ephemeral_index_builds = 1`;
- after advice: identical logical result with `ephemeral_index_builds = 0`;
- empty workload evicts only the advisor-owned statistic.

Additional hostiles verify:

- byte budget 0 blocks publication;
- explicit manual installation pins an already managed statistic;
- a second identical advisor run does not republish the runtime root;
- an existing exact persisted Join access path prevents redundant statistics materialization.

## Problem B — Pass57 outer physical catalogs still copied BTreeMap metadata

Pass57 made heavy physical derivative payloads COW/`Arc`, but the outer `PhysicalStore` directories remained ordinary value-owned `BTreeMap<K, Arc<State>>` / `BTreeSet` containers. Every candidate `PhysicalStore::clone()` therefore still copied catalog metadata in O(number of physical artifacts), even when all payloads themselves were shared.

This is historical OPEN #9 in Pass66.

## Implementation B — COW outer `PhysicalStore` roots

Pass67 moves all outer physical directories behind shared COW roots:

- installed relation directory;
- persisted I64 index directory;
- semantic-index directory;
- Γ-QCN endpoint-factor directory;
- Γ-QCN support directory;
- semantic-statistics directory;
- advisor-ownership set.

Mutation is centralized through private `Arc::make_mut` accessors. More importantly, relation-delta maintenance checks whether a family is actually affected **before** detaching its outer catalog root. A delta that touches only an installed relation and its I64 index therefore does not copy semantic-index/QCN/statistics/ownership metadata.

Logical/semantic authority is unchanged: these roots remain reconstructible physical state.

## Hostile evidence B

`physical_store_clone_cow_isolates_relation_mutation` now checks `Arc::ptr_eq` on all seven outer roots.

Immediately after clone all roots are shared. After a relation delta with one relevant persisted I64 index:

- relation catalog root detaches;
- I64 catalog root detaches;
- semantic-index, quotient-factor, quotient-support, statistics and ownership roots remain pointer-identical;
- source payload/index semantics remain unchanged while the candidate sees the exact delta.

A matched release diagnostic on the existing 200k-row fixture observed Pass66 → Pass67 average clone timings of approximately `1128 ns → 42 ns` for relation-only state and `218 ns → 40 ns` with one persisted index. These tiny timings are noisy microbenchmark evidence only; the architectural closure is the pointer-sharing invariant, not a universal speedup claim.

## CLOSED exactly in Pass67 — 2 concrete production problems

1. **Direct primitive-Join semantic statistics were manual-only.** They now have conservative create/retain/reuse/evict/manual-pin/budget/publication semantics tied to a planner/executor decision that demonstrably consumes the artifact.
2. **Historical OPEN #9: outer `PhysicalStore` artifact-map metadata was copied on every store snapshot.** The outer catalogs/ownership set are now COW roots and untouched families remain shared through relation-delta candidates.

The first closure advances, but does not fully close, historical multi-family lifecycle OPEN #3. The second closure fully removes one historical ledger item.

## Historical OPEN accounting

Pass66 ended with **24 historical active OPEN**.

- Historical 24 fully closed this pass: **1 / 24** — old #9 persistent outer physical artifact catalogs.
- Genuinely new OPEN: **0**.
- Total active OPEN after Pass67: **23**.

## Advanced but still OPEN

- **Multi-family physical lifecycle/advisor:** semantic indexes, Γ-QCN endpoint factors, persisted I64 indexes and now direct-Join semantic statistics have production lifecycle laws. Γ-QCN support, algebraic/future layouts, multiway counterfactual statistics, write-rate maintenance cost and autonomous telemetry remain outside one complete policy.
- **Logical fine-grained change calculus:** Pass66 removed full logical snapshot cloning, but first mutation of logical COW `BTreeMap` metadata and whole touched-relation reconstruction remain distinct from the now-closed physical outer-catalog clone problem.
- **Exact memory accounting:** deterministic retained-byte budgeting remains a planning estimate, not allocator/RSS truth.

## Active OPEN after Pass67 — 23

1. Structural/custom-equivalence physical indexing: durable recursive-key encoding/versioning, persisted structural indexes, arbitrary/plugin canonical laws and structural ordering.
2. General nested/multiway/bushy Join planning beyond bounded 3–8-leaf Γ-QCN.
3. Complete multi-family physical lifecycle beyond currently managed semantic indexes, Γ-QCN endpoint factors, persisted I64 indexes and conservative direct-Join semantic statistics.
4. Autonomous workload telemetry/read-write rates/decay/hysteresis/lifecycle scheduling.
5. Exact allocator/RSS accounting, external memory pressure and rebuild scheduling.
6. Canonical-key/cache encoding-version migration and compatibility law.
7. Remaining physical layouts plus explicit `OrderedView`/pagination.
8. Secondary-index/alternate-layout rebuild economics and fast physical reconstruction after recovery.
9. Durable revision DAG / branch+merge ancestry and merge replay.
10. General historical durable-format migration framework.
11. Arbitrary/plugin semantic executable artifact packaging/signing/authentication/deployment.
12. Transaction intent/outcome retention + GC; corrected Program7 remains external R&D.
13. Streaming/chunked checkpoints and metadata.
14. Real machine power-loss assurance plus Windows/network-FS/FUSE durability semantics.
15. General lock-poison/restart policy.
16. Group commit / async durability.
17. Replication / consensus and broader distribution architecture.
18. Durable-store authentication/MAC; CRC32C is corruption detection only.
19. Formal power-loss proof for rename/fsync/GC protocol.
20. Transaction repair runtime.
21. Formal mechanization of remaining semantic/transport/retention/power-loss obligations.
22. Maintained I64 Group constant-factor gap.
23. Maintained I64 TopK constant-factor gap.

## Verification

Rust: `rustc 1.98.1 (48a229cea 2026-09-01)`.

Frozen-source final gate PASS:

- `cargo fmt --all -- --check`;
- `cargo check --workspace --all-targets`;
- `cargo test --workspace --all-targets`;
- `cargo clippy --workspace --all-targets -- -D warnings`;
- `cargo test --workspace --all-targets --release`;
- `cargo build --workspace --release`;
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps`;
- `RUSTFLAGS='-C overflow-checks=yes' cargo test --workspace --all-targets --release`.

A pre-freeze cold overflow build hit the external execution timeout and was not counted; its warmed completion passed. The required post-freeze overflow gate also passed.

Additional hostile/pre-freeze evidence:

- full workspace debug tests PASS;
- `kernel-plan` PASS with `RUST_TEST_THREADS=1` and `16`;
- targeted release statistics-advisor and catalog-COW tests PASS.

Static frozen-source snapshot:

- **421 declared tests**;
- **165 `kernel-plan` tests**;
- **21 crates**;
- **59,205 Rust LOC**;
- **0 external registry/git Cargo sources**;
- **0 `unsafe` hits**;
- **19 existing `#[allow(...)]`**, no new suppression;
- **0 TODO/FIXME/todo!/unimplemented! hits**.

Production source diff relative to Pass66 is one file: `crates/kernel-plan/src/lib.rs`.

## Next frontier

If corrected Program7 arrives, hostile-review it against Pass67 first. Otherwise the next mainline choice should not fabricate a Γ-QCN-support benefit law. The strongest remaining branches are either counterfactual/multiway advisor scoring, general join planning, or structural persisted-key/version discipline; choose based on which can be given an exact consumer/cost contract in the next pass.
