# CFMD Pass51 Report — Maintained Γ-QCN Support State via Exact Change Fallback

**Status:** VERIFIED on Rust 1.98.1.

**Scope:** Γ-QCN derived-state maintenance only. Pass51 does not change logical query semantics, semantic authority, transaction authority, durable authority, or the Pass49 quotient law.

Source work started at **2026-09-20 18:31:56 +03:00**. Functional production logic froze at **18:43:33** (11m37s). Strict rustdoc later found one documentation-only invalid-HTML warning; only that doc comment was corrected and final source bytes froze at **18:44:56**, still 13m00s from the original start and below the 20-minute source limit.

## 1. Problem

Pass50 moved Γ-QCN law compilation into prepared `(query,Γ)` metadata and canonical endpoint keys into delta-maintained physical factors, but three expensive derived objects were still rebuilt on every execution:

1. N-way common quotient-key domains;
2. per-leaf row support masks;
3. the cross-coordinate monotone support fixed point.

This meant repeated reads of an unchanged revision still paid the constraint-propagation cost even after every canonical endpoint key had already been materialized.

The architectural requirement was stricter than "add a cache": the derived support graph must remain reconstructible physical state and must update through the same atomic relation-change boundary rather than becoming an independent invalidation/source-of-truth system.

## 2. Exact derivative boundary

Pass51 adds a dedicated reconstructible `semantic_quotient_supports` family to `PhysicalStore`.

A support state is bound to:

- the prepared Γ-QCN quotient specification;
- the participating `(relation, layout, width)` leaves;
- the exact pinned `SemanticContext`;
- the current logical stable-handle vectors of every leaf.

It contains the already-built quotient constraints and the converged base support masks.

This is intentionally modeled at the existing change-calculus correctness level:

```text
Fine relation delta
      |
      v
maintained canonical quotient factors
      |
      v
Dsupport_replace = Replace(fresh exact Γ-QCN fixed point)
```

In other words, Pass51 implements the universal exact `Replace` derivative for support state. It does **not** pretend that local fine propagation through the support graph has already been solved.

## 3. Prepared materialization and execution reuse

`PreparedPlan::materialize_semantic_quotient_support(...)`:

1. ensures the Pass50 canonical quotient factors exist;
2. builds the exact common-domain/support fixed point once;
3. stores it as reconstructible physical state;
4. publishes it through the ordinary physical transition epoch;
5. is idempotent when the exact state is already present.

Prepared execution first looks for a support state with:

```text
same quotient binding
same pinned SemanticContext
same current stable-handle vectors
```

Only then is it consumed. Otherwise execution falls back to ordinary fresh Γ-QCN construction.

`ExecutionStats.multiway_join_maintained_quotient_support_hits` makes this path observable.

A support hit bypasses both canonical endpoint-key reconstruction and the support fixed-point computation on the read path.

## 4. Atomic relation-delta maintenance

The existing physical relation transition now has the following relevant ordering:

```text
prevalidate relation + all affected derived families
        |
        v
apply installed-relation edit
        |
        v
apply I64 / semantic-index / quotient-factor / statistics deltas
        |
        v
recompute affected Γ-QCN support state from maintained factors
        |
        v
publish one transition epoch
```

Because relation transitions are prepared on a candidate store before publication, a failure during support rebuilding discards the candidate rather than exposing a partially updated derived graph.

Relation reinstall invalidates every support state that references that exact relation/layout.

## 5. Hostile / regression evidence

The existing Pass50 three-relation exact-I64 Γ-QCN fixture was extended to attack the new boundary.

It verifies:

1. fresh prepared execution first reuses all 140 delta-maintained canonical endpoint keys;
2. support materialization succeeds exactly once and a repeated materialization is a no-op;
3. the next execution matches the fresh logical reference;
4. the next execution records a maintained-support hit and performs zero maintained quotient-key lookups because the fixed point itself is already materialized;
5. an invalid semantic delta is rejected and the entire `PhysicalStore` — relation, factors and maintained support included — remains exactly unchanged;
6. a valid relation delete+insert is applied through the ordinary atomic relation-delta transition;
7. the affected support state is rebuilt inside that transition from the already-maintained quotient factors;
8. post-delta prepared execution again matches fresh logical reference evaluation and records a maintained-support hit without rebuilding endpoint keys on the read path;
9. the targeted test passes in debug and release.

No new Clippy suppression was added.

## 6. Important non-claims

Pass51 is **not** fine-grained incremental arc-consistency propagation.

Current write path for an affected support program is:

```text
fine input delta -> exact Replace of the dependent support fixed point
```

Therefore:

- repeated reads can reuse the fixed point;
- correctness and atomicity are closed;
- write amplification for large support programs remains OPEN;
- a multi-relation revision can rebuild the same support program more than once while constructing the unpublished candidate root;
- local queue-based deletion/insertion propagation, support counters and batch coalescing are not yet implemented.

This distinction is deliberate: logical completeness / incremental correctness is now established at the support-state boundary; incremental efficiency remains a separate optimization problem.

## 7. External R&D status

An independent R&D agent reported promising work on structural canonical equality keys, structural `MaterializedSetSupportState`, fixpoint adjacency lowering and several other generalization-tax sites.

None of that code is integrated in Pass51. It remains **R&D evidence / production OPEN** until the promised patch/ZIP/raw artifacts are received, hostile-reviewed against the current Pass51 workspace, and manually adapted if sound. This avoids mechanically merging a parallel physical design into the authoritative branch.

## 8. Verification

Final Rust 1.98.1 gate on the final source bytes:

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

The first overflow-check invocation timed out externally while recompiling under the alternate `RUSTFLAGS`; it was treated as neither PASS nor FAIL. The warmed repeat completed successfully.

Workspace metrics:

- 357 declared Rust tests;
- 129 `kernel-plan` tests (128 normal + 1 ignored diagnostic benchmark);
- 21 crates;
- 49,003 Rust LOC under `crates/`;
- 19 pre-existing `#[allow(...)]` sites, no new suppression;
- 0 external Cargo registry/git sources;
- 0 `unsafe` in `crates/`.

## 9. Problem ledger

### CLOSED exactly in Pass51 — 2

1. ✅ **Prepared Γ-QCN still recomputed its common-domain/support fixed point on every execution.** The exact converged constraint/support state can now be explicitly materialized and reused by prepared execution under exact context + stable-handle coherence checks.
2. ✅ **There was no atomic Change-boundary for maintained Γ-QCN support state.** Relation deltas now maintain affected support programs in the same unpublished physical candidate transition, using the exact `Replace` derivative over the already-maintained canonical quotient factors; invalid deltas leave the complete store unchanged.

### Historical OPEN from Pass50 fully closed this pass

**0 / 22.** Pass51 closes the correctness/reuse boundary for maintained support state, but the historical multiway/multi-family item also includes fine-grained propagation efficiency, lifecycle policy, structural/custom canonical factors, unbounded planning and other broader work.

### Advanced but still OPEN

1. 🟨 **Fine Γ-QCN Change/Dq maintenance.** Exact `Replace` maintenance is production-integrated; local support-count/queue propagation for small deltas remains OPEN.
2. 🟨 **Batch revision maintenance.** A multi-relation candidate can rebuild one support program after more than one constituent relation edit. Coalescing to one derivative per complete logical revision remains OPEN.
3. 🟨 **Γ-QCN lifecycle/advisor.** Support state and quotient factors are explicit reconstructible families, but create/share/retain/evict/rebuild policy and byte/RSS accounting are not yet integrated with the multi-family advisor.
4. 🟨 **Structural/custom canonical laws.** Pass51 continues to use the production primitive canonical-factor fragment. The independent structural-key R&D has not yet been integrated or verified against this branch.
5. 🟨 **I64 Group/TopK constant-factor debt.** Unchanged by Pass51.

### Historical / active OPEN after Pass51 — 22

1. ⬜ Structural/custom-equivalence canonical indexing and typed production.
2. ⬜ General nested/multiway/bushy Join planning beyond the verified bounded 3–8-leaf builtin-primitive Γ-QCN fragment: adaptive/unbounded search, richer non-equality predicate graphs and fully general indexed/typed subset execution.
3. ⬜ First-class multi-family physical access advisor/lifecycle across specialized I64, generic semantic indexes, retained statistics, Γ-QCN factors/support state and future layouts.
4. ⬜ Autonomous workload statistics/telemetry, retention decay and lifecycle scheduling.
5. ⬜ Exact physical-index/statistics/quotient-factor/support-state byte/resident-memory budgeting and rebuild scheduling.
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

### New OPEN created in Pass51

**0.** Fine support propagation and revision-level coalescing are refinements of the existing Γ-QCN incremental-maintenance frontier rather than independent new subsystems.

## 10. Result / next direction

Pass51 establishes the clean correctness boundary needed before attempting a more aggressive derivative:

```text
semantic authority: Revision=(S,Γ,M)
            |
            v
canonical quotient factors     // exact fine-delta maintained
            |
            v
Γ-QCN support state            // exact Change/Replace maintained
            |
            v
original-order enumeration + exact Γ predicate recheck
```

The next useful experiment is no longer "can support be maintained?" — it can. The question is whether the Replace derivative can be refined into a local exact derivative with support counters / dependency queues and revision-batch coalescing **without** creating bespoke invalidation semantics. If that refinement becomes ad-hoc, the Pass51 Replace boundary should remain the correctness implementation.
