# CFMD Pass47 Report — Physical Join Provenance / Exact Order Restoration / First Non-Contiguous Permutation

**Status:** VERIFIED on Rust 1.98.1.

**Production source window:** 2026-09-20 16:06:11 → 16:25:18 +03:00 (**19m07s**). Production source was frozen at 16:25:18; the final verification gate found no correctness defect, so the freeze was not lifted.

**Scope:** physical Join planner/execution layer only. Pass47 does not change semantic `Revision=(S,Γ,M)`, exact query semantics, transaction authority, or durable authority.

## 1. Problem

Pass46 deliberately refused arbitrary leaf permutation because the current exact relational evaluator gives `JoinEq` a deterministic left-major/right-minor Bag row-production order. Reassociating while preserving leaf order was safe; permuting leaves was not.

Two connected physical gaps remained:

1. there was no explicit provenance/order-restoration mechanism capable of carrying original leaf scan identity through a reordered intermediate Join and reconstructing the exact reference output order;
2. consequently, even when retained statistics showed that a non-contiguous pair should be joined first, the planner could not safely execute that permutation without changing observable Bag row order and output column order.

Pass47 attacks this correctness prerequisite directly rather than treating physical Join commutativity as semantic commutativity.

## 2. Physical Join provenance and order restoration

Pass47 introduces a correctness-first physical provenance row used only inside the reordered multiway path:

```text
ProvenanceJoinRow {
    fragments[original_leaf],
    scan_ordinals[original_leaf],
}
```

Each base leaf is read in authoritative logical scan order from `PhysicalStore`; its original zero-based scan ordinal is retained as physical provenance. Reordered joins merge only disjoint leaf fragments and continue to evaluate every admitted equality predicate through pinned Γ.

After the reordered physical tree finishes:

1. rows are sorted lexicographically by the vector of **original leaf scan ordinals**;
2. row fragments are flattened in the **original logical leaf order**.

For the currently admitted equality fragment this reproduces the reference evaluator's nested left-major/right-minor order rather than exposing the physical reordered join sequence.

`ExecutionStats.multiway_join_order_restorations` makes admission observable without making provenance semantic authority.

## 3. First safe non-contiguous leaf permutation

Pass47 verifies the first leaf-permuting physical path for a three-leaf primitive equality Join graph.

Current admission is intentionally narrow:

- exactly three base relation leaves;
- current primitive equality predicates admitted by the existing multiway flattening rules;
- retained compatible semantic statistics must exist for the relevant equality edges;
- the path declines when a relevant persisted semantic/I64 index already exists, preserving the established indexed planner family;
- a non-contiguous `(leaf0, leaf2)` first join is chosen only when its complete estimated work is lower than the best adjacent alternative.

The complete estimate includes:

```text
first_join_work
+ estimated_third_join_work
+ estimated_order_restoration_sort_work
```

so permutation does not get a free accounting pass for the restoration sort.

This is a first verified non-contiguous permutation, **not** a claim that arbitrary N-way bushy planning is solved.

## 4. Hostile / regression evidence

Pass47 verifies:

- a synthetic provenance fixture with distinguishable row fragments is restored by original scan ordinals rather than physical join order;
- a real three-relation Bag query executes a non-contiguous first pair and produces exactly the same rows, multiplicities, columns and row order as the logical reference evaluator;
- duplicate/skew behavior remains exact under pinned Γ predicates;
- existing persisted-index multiway execution is not intercepted by the new correctness-first permutation path;
- the previous no-retained-statistics path is not intercepted;
- restoration work is charged in the candidate estimate;
- debug/release execution agrees;
- no new `#[allow(...)]` site was introduced relative to Pass46 (19 existing sites in both checkpoints);
- no `unsafe`, TODO/FIXME, `todo!` or `unimplemented!` was introduced;
- no side-effecting mutation is hidden inside assertion-only code.

Freeze result in `kernel-plan`: **120 passed, 0 failed, 1 ignored diagnostic benchmark**.

## 5. Verification

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

The first overflow-check invocation hit the external execution timeout while recompiling under the changed `RUSTFLAGS`; it was not counted as PASS or FAIL. The same command was rerun on the warmed overflow target and completed PASS.

Workspace metrics at freeze:

- 349 declared Rust tests;
- 121 `kernel-plan` tests (120 normal + 1 ignored diagnostic benchmark);
- 21 crates;
- 47,310 Rust LOC under `crates/`;
- 0 external Cargo registry/git sources;
- 0 `unsafe` in `crates/`;
- 0 TODO/FIXME/todo!/unimplemented! markers in `crates/`.

Evidence is retained under `evidence/pass47/` inside the workspace.

## 6. Problem ledger

### CLOSED exactly in Pass47 — 2

1. ✅ **No explicit physical provenance/order-restoration boundary existed for safe Join leaf permutation.** Pass47 carries original leaf fragments and authoritative logical scan ordinals through reordered joins, then restores both original leaf column order and exact reference Bag row-production order before the result crosses the physical boundary.
2. ✅ **The planner could not safely execute even a profitable non-contiguous three-way leaf permutation.** The current three-leaf primitive equality fragment can now choose `(leaf0, leaf2)` first from retained statistics, account for restoration cost, execute the reordered tree, and return output identical to logical reference evaluation.

### Historical OPEN from Pass46 fully closed this pass

**0 / 22.** The broad general multiway/bushy item is materially advanced but still includes arbitrary N-way subset enumeration, indexed/typed permuted execution and richer predicate graphs.

### Advanced but still OPEN

1. 🟨 **General nested/multiway/bushy Join planning.** Pass47 proves one real non-contiguous three-way permutation with exact order restoration. Arbitrary N-way subset/bushy enumeration and broader predicate graphs remain OPEN.
2. 🟨 **Order-restoring execution representation.** The new provenance path is correctness-first and currently carries/clones logical `Row` fragments and performs a final sort. Stable-handle/native typed provenance, batch composition, indexed probes inside permuted plans and lower restoration tax remain OPEN physical work.
3. 🟨 **Statistics ecosystem.** Retained exact cardinality is sufficient for the current admitted path; histograms/correlation/autonomous telemetry remain OPEN.
4. 🟨 **Multi-family lifecycle advisor and I64 Group/TopK constant-factor debt.** Unchanged by Pass47.

### Historical / active OPEN after Pass47 — 22

1. ⬜ Structural/custom-equivalence canonical indexing and typed production.
2. ⬜ General nested/multiway/bushy Join planning beyond the verified three-way non-contiguous order-restored primitive fragment: arbitrary N-way subset enumeration, indexed/typed permuted execution and richer predicate graphs.
3. ⬜ First-class multi-family physical access advisor/lifecycle across specialized I64, generic semantic indexes, retained statistics and future layouts.
4. ⬜ Autonomous workload statistics/telemetry, retention decay and lifecycle scheduling.
5. ⬜ Exact physical-index/statistics byte/resident-memory budgeting and rebuild scheduling.
6. ⬜ Canonical-key encoding/version migration for long-lived physical caches/statistics.
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

### New OPEN created in Pass47

**0.** The correctness-first logical-row provenance representation and remaining N-way/indexed work refine the already-open general multiway/physical-performance frontier rather than introducing an independent architectural obligation.

## 7. Rejected / deliberately deferred routes

- no assumption that equality Join is freely commutative when physical Bag row order is observable;
- no leaf permutation without an explicit restoration proof/path;
- no optimizer credit that ignores final restoration work;
- no replacement of pinned Γ equality with host equality while carrying provenance;
- no interception of established persisted-index plans by the initial correctness-first permutation path;
- no claim that three-way permutation equals arbitrary general bushy planning;
- no new lint suppression.

## 8. Result / next direction

Pass47 removes the correctness blocker identified in Pass46: relation permutation can now be a physical optimization without leaking reordered row/column order, provided the execution path carries sufficient provenance and restores the reference contract.

The next clean step is to generalize the verified mechanism into an N-way subset/bushy candidate representation while replacing logical-row provenance with compact stable-handle/typed/native provenance and integrating the existing `JoinAccessDecision` families inside permuted plans.
