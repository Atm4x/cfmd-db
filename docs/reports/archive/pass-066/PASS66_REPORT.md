# PASS66 REPORT — persistent logical COW + persisted-I64 lifecycle

Status: **VERIFIED after final Rust and packaging gates**.

Authoritative base: verified Pass65. Program7 was still external R&D and was not used by this pass.
Production source freeze: **2026-09-20 23:50:16 UTC**. `crates/` SHA-256 at freeze is byte-identical to the post-gate snapshot.

## Problem A — Pass65 incremental compiler still cloned the logical database

Pass65 removed the second full revision compiler from certified relation-only transitions, but `Revision::relation_update_candidate()` still started with `self.state.clone()`. `DatabaseState`, `FiniteModel` and every relation payload were value-owned `BTreeMap`/`Vec` structures, so a tiny relation transition copied the complete logical snapshot before the incremental compiler could help.

This was the only genuinely new OPEN formalized by Pass65 (#25).

## Implementation A — persistent/COW logical roots

Pass66 changes logical storage representation without changing logical semantics or revision authority:

- `CowValue<T>` stores large value roots behind `Arc<T>` and detaches through `Arc::make_mut`;
- `CowMap<K,V>` gives carriers/fields a shared immutable root with ordinary value/mutation semantics;
- `RelationStore` stores a shared relation directory whose relation payloads are independently shared `SharedRelationRows(Arc<Vec<Row>>)` roots;
- `DatabaseState.lifecycle` is COW-backed;
- existing normalization, transport, query, revision and durability code continues to consume the same extensional logical state.

A clone now shares carriers, fields, lifecycle, relation-directory and every relation payload. Mutating one relation detaches the relation-directory metadata and the touched payload only; untouched relation payloads remain pointer-identical shared roots.

This does **not** claim a fully persistent asymptotically optimal map. First mutation of the relation directory still copies `BTreeMap` metadata proportional to the number of relations, and the current relation-data calculus can still reconstruct the touched relation as a whole. The closed problem is the Pass65 full logical-payload snapshot clone, not the later typed `D(Change)` frontier.

## Hostile evidence A

The production test `database_state_clone_path_copies_only_the_mutated_relation` checks `Arc::ptr_eq` directly:

- source and candidate initially share carriers, fields, lifecycle, relation-directory and row payloads;
- after mutation, source is unchanged;
- touched relation payload and relation-directory root detach;
- unrelated relation payloads remain shared.

The same 40 relations × 5,000 rows diagnostic shape used for the Pass65 clone finding produced:

- `DatabaseState::clone()` median: **30 ns**;
- clone + replace one relation root median: **190 ns**;
- Pass65 deep-clone reference: **4,330,525 ns**.

These nanosecond measurements are microbenchmark evidence only. The structural-sharing test is the architectural proof; Pass66 does not turn these timings into an SLA or claim complete O(|Δ|) transaction construction.

## Problem B — persisted I64 indexes remained manual-only

Pass63 introduced one cross-family inventory/memory budget and Pass64 added Γ-QCN factor lifecycle, but persisted exact-I64 indexes were still outside the create/retain/evict advisor even though:

- their build cost is explicit;
- retained-byte accounting already exists;
- direct I64 Join actually consumes them.

## Implementation B — conservative I64 lifecycle advisor

Pass66 adds advisor-owned persisted-I64 lifecycle under the existing physical-artifact policy:

- repeated direct Join workload may create, retain, reuse or evict exact single-key I64 indexes;
- only advisor-owned I64 artifacts are evictable;
- explicit/manual `install_i64_index` removes advisor ownership and therefore pins the artifact;
- managed/global retained-byte budgets are enforced;
- one-shot work is rejected when build work does not amortize;
- unprofitable work is rejected **before** candidate-index construction, so the advisor does not pay the O(n) build it just decided not to retain;
- runtime and durable-runtime wrappers publish one new physical root only when the advisor actually changes physical state.

### Hostile correction: only count operators that consume the family

An initial version reused generic semantic-index Filter observations. The hostile test showed the advisor could create a persisted I64 index while `FilterEqConst` still executed through another physical family: the artifact existed but the operator never read it.

That route was rejected. Production I64 advice is admitted only from Join shapes for which the existing persisted-I64 path is an actual executor access path. The end-to-end hostile verifies that repeated Join advice creates the artifact and subsequent execution records a persisted-index hit; empty workload evicts advisor-owned state, while a manual index survives.

## CLOSED exactly in Pass66 — 2 concrete production defects

1. **Full logical `DatabaseState` deep clone before a source-bound relation candidate.** Logical roots and per-relation payloads are COW/shared; the Pass65 new OPEN #25 is closed.
2. **Persisted exact-I64 indexes were inventory-only/manual-only from the unified lifecycle perspective.** They now have conservative create/retain/reuse/evict/manual-pin/budget semantics tied to an executor path that actually consumes them.

The second closure advances, but does not fully close, historical multi-family lifecycle OPEN #3.

## Historical OPEN accounting

Pass65 ended with **25 active OPEN** = 24 historical items + 1 newly formalized logical-state clone item.

- Historical 24 fully closed this pass: **0 / 24**.
- Pass65-new OPEN #25 fully closed this pass: **1 / 1**.
- Genuinely new OPEN: **0**.
- Total active OPEN after Pass66: **24**.

## Advanced but still OPEN

- **Multi-family physical lifecycle/advisor:** semantic indexes, Γ-QCN endpoint factors and persisted I64 indexes now have production lifecycle laws. Statistics, Γ-QCN support, algebraic/future layouts, counterfactual path-shaping, write-rate maintenance cost and autonomous telemetry remain outside one complete policy.
- **Logical fine-grained change calculus:** cloning the database root is no longer the dominant snapshot tax, but first mutation of ordinary COW BTreeMap metadata and whole touched-relation reconstruction are not a general persistent-map / typed `D(Change)` solution.
- **Exact memory accounting:** deterministic retained-byte budgeting remains a planning estimate, not allocator/RSS truth.

## Active OPEN after Pass66 — 24

1. Structural/custom-equivalence physical indexing: durable recursive-key encoding/versioning, persisted structural indexes, arbitrary/plugin canonical laws and structural ordering.
2. General nested/multiway/bushy Join planning beyond bounded 3–8-leaf Γ-QCN.
3. Complete multi-family physical lifecycle beyond currently managed semantic indexes, Γ-QCN endpoint factors and persisted I64 indexes.
4. Autonomous workload telemetry/read-write rates/decay/hysteresis/lifecycle scheduling.
5. Exact allocator/RSS accounting, external memory pressure and rebuild scheduling.
6. Canonical-key/cache encoding-version migration and compatibility law.
7. Remaining physical layouts plus explicit `OrderedView`/pagination.
8. Secondary-index/alternate-layout rebuild economics and fast physical reconstruction after recovery.
9. Persistent outer physical artifact-map metadata instead of O(number of artifacts) map clones.
10. Durable revision DAG / branch+merge ancestry and merge replay.
11. General historical durable-format migration framework.
12. Arbitrary/plugin semantic executable artifact packaging/signing/authentication/deployment.
13. Transaction intent/outcome retention + GC; corrected Program7 remains external R&D.
14. Streaming/chunked checkpoints and metadata.
15. Real machine power-loss assurance plus Windows/network-FS/FUSE durability semantics.
16. General lock-poison/restart policy.
17. Group commit / async durability.
18. Replication / consensus and broader distribution architecture.
19. Durable-store authentication/MAC; CRC32C is corruption detection only.
20. Formal power-loss proof for rename/fsync/GC protocol.
21. Transaction repair runtime.
22. Formal mechanization of remaining semantic/transport/retention/power-loss obligations.
23. Maintained I64 Group constant-factor gap.
24. Maintained I64 TopK constant-factor gap.

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

The first cold overflow-check invocation hit the external execution timeout and was not counted. The warmed retry completed with exit 0.

Static snapshot before packaging:

- **417 declared tests**;
- **161 `kernel-plan` tests**;
- **21 crates**;
- **58,327 Rust LOC**;
- **0 external registry/git Cargo sources**;
- **0 `unsafe` hits**;
- **19 existing `#[allow(...)]`**, no new suppression;
- **0 TODO/FIXME/todo!/unimplemented! hits**;
- **0 `GenericScan` hits**.

## Next frontier

If corrected Program7 arrives, hostile-review it against Pass66 first. Otherwise continue the same production lifecycle program: semantic statistics are the next conservative family with a real consumer and explicit rebuild/retained cost; Γ-QCN support should wait for the stronger per-key support calculus rather than receive a fabricated benefit model.
