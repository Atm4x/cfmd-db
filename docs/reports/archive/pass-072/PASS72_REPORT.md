# CFMD Pass72 Report — Program8 Quotient Hypergraph Engine integration

Date: 2026-09-21
Status: **VERIFIED**
Authoritative base: verified Pass71
Production source freeze: **2026-09-21 10:00:42 UTC**

## Goal

Hostile-review and integrate R&D Program8 on top of Pass71 without regressing Pass69 key-format/binding laws, Pass70 compiled Γ, or Pass71 durable artifact recovery.

## Result

Program8 is production-integrated after semantic rebase.

The Γ-QCN path now maintains live quotient-key support incrementally, can derive an execution order from a GYO-reducible normalized quotient hypergraph, lifts the previous eight-leaf ceiling for the certified acyclic quotient branch, and delays full row materialization until a complete quotient assignment survives.

For 3..=8 leaves, the existing subset-DP cost gate remains authoritative. For >8 leaves, QCN is admitted only when quotient edges cover every leaf and the normalized quotient hypergraph is GYO-reducible. Cyclic or incomplete quotient systems fall back to the previous planner/executor.

## Problem → hypothesis → implementation → falsification → result

### 1. Repeated support rediscovery

**Problem.** QCN support propagation repeatedly rescanned leaf buckets to rediscover viable quotient keys.

**Hypothesis.** Maintain per-key live leaf support counters, with duplicate-row-safe per-leaf row counts.

**Implementation.** `SemanticQuotientLeaf` retains `live_rows_by_key`; `SemanticQuotientConstraint` retains static `key_leaf_support` plus dynamic `live_key_leaf_support`. Monotone deletions decrement leaf support only when the last live duplicate for that leaf/key disappears.

**Falsification.** Duplicate-row hostile removes one duplicate, then the last duplicate, and checks support transitions exactly once.

**Result.** Viability is driven by maintained support counts rather than repeated all-leaf bucket rescans.

### 2. Artificial >8 QCN ceiling

**Problem.** The previous order-preserving QCN path depended on bounded subset DP and stopped at eight leaves.

**Hypothesis.** Use the quotient hypergraph itself to derive an execution order when it has a certified acyclic reduction.

**Implementation.** `quotient_hypergraph_search_order()` performs GYO-style subset/duplicate-edge reduction plus degree<=1 leaf elimination. The prepared quotient program records the resulting search order.

**Falsification.** Chain hypergraphs succeed; explicit cycles fail. Nine-way left-deep and bushy logical trees compare exactly with the logical evaluator. A syntactically cyclic predicate graph that collapses under pinned Γ to one certified common quotient remains valid and exact.

**Result.** The certified GYO-reducible quotient branch is no longer artificially bounded at eight leaves.

### 3. QCN row materialization

**Problem.** QCN key construction/enumeration materialized more row structure than needed before a full assignment was known.

**Hypothesis.** Read endpoint values by stable row handle and carry ordinals until a complete assignment survives.

**Implementation.** Key construction uses `leaf_value_by_handle`; enumeration carries ordinals. Completed assignments are sorted by logical leaf ordinals before final row materialization.

**Falsification.** Nine-way hostiles compare exact bag order and multiplicity against the logical evaluator before and after maintained-support relation deltas.

**Result.** QCN endpoint extraction/enumeration is late-materialized without changing logical order/multiplicity.

### 4. Integration hostile: retained-byte accounting

**Problem.** Program8's new support-counter maps were absent from the existing physical retained-byte estimator.

**Hypothesis.** Global memory-budget law must include every newly retained support structure.

**Implementation.** Pass72 extends quotient-support byte estimation over `live_rows_by_key`, `key_leaf_support`, and `live_key_leaf_support`, including canonical-key heap bytes and counter storage.

**Result.** Program8 no longer silently undercounts managed physical memory.

## CLOSED exactly in Pass72 — 5 narrower production problems

1. repeated all-leaf viable-key rediscovery on maintained QCN support;
2. duplicate rows incorrectly threatening leaf-support semantics without explicit row counts;
3. no certified quotient-hypergraph execution order;
4. artificial >8 ceiling for fully covered GYO-reducible Γ-QCN programs;
5. eager full-row materialization inside the QCN enumeration path.

## Historical OPEN accounting

Pass71 ended with **22 active historical OPEN**.

- Historical OPEN fully closed in Pass72: **0 / 22**.
- Genuinely new OPEN: **0**.
- Total active OPEN after Pass72: **22**.

The historical general multiway/bushy item remains OPEN because Program8 explicitly does not solve arbitrary cyclic/non-GYO planning or general hypertree-width/worst-case-optimal joins.

## Verification

Rust: `rustc 1.98.1 (48a229cea 2026-09-01)`.

Frozen-source final gate PASS:

- `cargo fmt --all -- --check`;
- `cargo check --workspace --all-targets`;
- `cargo test --workspace --all-targets` — **436 passed / 0 failed / 8 ignored**;
- `cargo clippy --workspace --all-targets -- -D warnings`;
- `cargo test --workspace --all-targets --release` — **436 passed / 0 failed / 8 ignored**;
- `cargo build --workspace --release`;
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps`;
- `RUSTFLAGS='-C overflow-checks=yes' cargo test --workspace --all-targets --release` — **436 passed / 0 failed / 8 ignored**.

Cold release and overflow-check compilation exceeded external command windows and were not counted. Warmed reruns completed successfully. Freeze/post-gate `crates/` SHA-256 inventories are byte-identical.

Frozen snapshot:

- **444 declared tests**;
- **177 `kernel-plan` declared tests**;
- **21 crates**;
- **62,901 Rust LOC**;
- **0 external registry/git Cargo sources**;
- **0 unsafe hits**;
- **19 existing `#[allow(...)]`**, no new suppression;
- **0 TODO/FIXME/todo!/unimplemented! hits**.

Production source diff relative to Pass71:

- `crates/kernel-plan/src/lib.rs`.

## Next frontier

Program8 removes the bounded-leaf ceiling only for a certified acyclic quotient branch. The main remaining planning frontier is genuinely general non-GYO/cyclic multiway planning and its cost law. On the durability side, Pass71 still leaves typed physical-layout recipes/I64 recovery open.
