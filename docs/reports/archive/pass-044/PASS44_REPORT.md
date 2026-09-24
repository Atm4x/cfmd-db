# CFMD Pass44 Report — Multiway Join Reassociation / Unified Join Access Costing

**Status:** VERIFIED on Rust 1.98.1.

**Scope:** physical planner/executor data-plane only. Pass44 does not change semantic authority, durable revision authority, Γ laws, transaction publication law, or the Pass43 semantic-index lifecycle ownership model.

## 1. Problem

Pass43 left the next connected planner cluster open. Source/hostile audit found three concrete defects inside that cluster:

1. scan-vs-index costing was not actually universal across the current I64 Join fast paths: legacy persisted/ephemeral I64 paths, including fused/batch paths, could bypass the Pass42 cost decision;
2. a profitable large primitive non-I64 equality Join without a persisted index had no costed transient canonical-index path;
3. nested equality Join trees could not reassociate, and an index on a later base relation could not be consumed once the left side was already an intermediate Join result.

## 2. Unified cost gating across current Join fast paths

`SemanticAccessCostModel` now exposes prospective ephemeral-join costing in addition to persisted-index costing.

Every current I64 Join route that can build/use an index is gated by the same work decision, including:

- ordinary direct Join;
- persisted specialized I64 Join;
- ephemeral specialized I64 Join;
- `Join -> Project` specialization;
- typed-batch/fused Join paths.

A tiny/unselective relation can therefore remain on exact scan even when a persisted I64 index exists, and rejection no longer falls through to an ephemeral I64 build.

Existing compatible generic semantic indexes are considered before building a new transient family. Candidate rows remain rechecked by the exact Γ equality law; no index becomes semantic authority.

## 3. Costed transient primitive semantic Join index

A large direct primitive equality Join without an installed compatible generic index may now build a transient `SemanticBucketIndex<CanonicalEqKey, ...>` when the estimated total probe+build work beats nested scan.

The key is derived only through the admitted primitive canonical-key law under pinned Γ. Structural/custom equivalence therefore remains exact fallback and receives no invented representation.

The transient index is execution-local derivative state. It is not installed into `PhysicalStore`, not retained by the Pass43 advisor, and not durable authority.

## 4. Contiguous multiway equality Join planner

Pass44 adds a correctness-first multiway planner for connected contiguous/in-order equality-Join trees.

It:

- extracts base leaves and equality edges from nested `JoinEq` plus `FilterEqColumns` predicates;
- uses interval dynamic programming to evaluate alternative parenthesizations while preserving leaf order;
- uses current relation/index statistics when estimating a merge;
- may choose a persisted right-side semantic or specialized I64 index at a merge boundary;
- remaps column coordinates when rebuilding a reassociated physical tree;
- executes a direct-right indexed probe even when the left input is already an intermediate result;
- keeps all returned candidates Γ-revalidated before output.

This is intentionally **not** a claim of arbitrary bushy/permuted join-order search. Leaf permutation, disconnected/general predicates, structural keys and richer cardinality estimation remain OPEN.

## 5. Hostile falsification

Pass44 verifies:

- an unprofitable persisted I64 index is rejected and does not trigger an ephemeral fallback;
- the same cost law applies to fused `Join -> Project` / typed-batch I64 fast paths;
- a large Text primitive Join can choose a profitable transient canonical semantic index and equals logical reference evaluation;
- a hostile `A ⋈ (B ⋈ C)` tree reassociates to the cheaper contiguous order;
- a persisted index is consumed on a later base relation after an intermediate Join has already been produced;
- multiway planning preserves cross-relation `FilterEqColumns`, Bag duplicates and logical output order;
- no new Clippy suppression was introduced to hide planner complexity; the new planner was split into explicit context/merge/probe helpers.

## 6. Diagnostic benchmark

Ignored release diagnostic on a deliberately hostile three-way fixture:

```text
baseline median = 5,025,458 ns
optimized median = 32,148 ns
ratio            = 156.322x
```

This workload is constructed so the original parenthesization creates a large avoidable intermediate. It is evidence that reassociation matters, **not** a universal 156x performance claim.

Evidence: `evidence/pass44/MULTIWAY_BENCH_RELEASE.log`.

## 7. Verification

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

Workspace metrics at freeze:

- 332 declared Rust tests;
- 104 `kernel-plan` tests (103 normal + 1 ignored diagnostic benchmark);
- 21 crates;
- 45,361 Rust LOC under `crates/`;
- 0 external Cargo registry/git sources;
- 0 `unsafe` in `crates/`.

## 8. Problem ledger

### CLOSED exactly in Pass44

1. ✅ **Current I64 Join fast paths could bypass scan-vs-index costing.** Persisted and ephemeral specialized I64 paths, including fused/batch specializations, now obey explicit access-work gating and cannot silently rebuild an index after a cost rejection.
2. ✅ **Large primitive non-I64 Join had no profitable transient canonical-index path.** Primitive Γ-certified canonical keys can now back an execution-local semantic index when its build+probe work beats scan.
3. ✅ **Contiguous nested equality Join trees could not be physically reassociated or probe a later indexed base relation after an intermediate Join.** Pass44 adds interval join reassociation plus intermediate-left/direct-right indexed execution, including `FilterEqColumns` edge preservation.

### Historical OPEN from Pass43 fully closed this pass

**0 / 22.** The broad historical item “nested/multiway/bushy Join planning and general join-order search” is materially advanced but not fully closed; arbitrary leaf permutation/general bushy search remains open. The broad multi-family physical advisor item is also not fully closed.

### Advanced but still OPEN

1. 🟨 **General multiway/bushy Join planning.** Contiguous/in-order equality trees are now planned; arbitrary relation permutation, disconnected predicate graphs, richer structural keys and general bushy search remain open.
2. 🟨 **Multi-family access planning.** Current Join paths now share cost gating and understand persisted generic/persisted specialized/transient families, but lifecycle/advisor ownership remains generic-semantic-index-only and there is not yet one first-class candidate representation for every physical family/layout.
3. 🟨 **Cardinality/selectivity estimation.** Current estimates deliberately use simple row/distinct-key statistics; correlations and multi-column histograms are absent.
4. 🟨 **I64 Group/TopK constant factors.** Existing Pass43 residual gaps remain open.

### Historical / active OPEN after Pass44 — 22

1. ⬜ Structural/custom-equivalence canonical indexing and typed production.
2. ⬜ General nested/multiway/bushy Join planning: arbitrary leaf permutation/non-contiguous plans/richer predicate graphs.
3. ⬜ First-class multi-family physical access advisor across specialized I64, generic semantic indexes and future layouts.
4. ⬜ Autonomous workload statistics/telemetry, retention decay and lifecycle scheduling.
5. ⬜ Exact physical-index byte/resident-memory budgeting and rebuild scheduling.
6. ⬜ Canonical-key encoding/version migration for long-lived physical caches.
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

### New OPEN created in Pass44

**0.** Pass44 refined the wording of existing planner debt but did not introduce a new architectural obligation.

## 9. Rejected / deliberately deferred routes

- no claim that one physical index family dominates every other family;
- no arbitrary relation permutation without a correctness-preserving column/predicate mapping model;
- no structural/custom canonical key invented for optimizer convenience;
- no persisted installation of an execution-local transient semantic index;
- no benchmark-derived universal threshold;
- no hidden Clippy suppression for oversized planner functions.

## 10. Result / next direction

Pass44 establishes the first real multiway physical reassociation slice and removes family-specific cost bypasses.

The next highest-value cluster is to make physical access candidates first-class across **scan / persisted specialized I64 / persisted generic semantic / transient specialized / transient generic**, then let both direct and multiway planning consume the same candidate interface. After that, arbitrary relation permutation and richer cardinality statistics can be added without duplicating family-specific execution logic.
