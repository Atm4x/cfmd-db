# PASS64 REPORT — Γ-QCN factor lifecycle advisor

Status: **SOURCE VERIFIED; standalone Pass64 packaging was superseded by Pass65**

Source window: **2026-09-20 22:16:42 UTC → 22:36:53 UTC (20m11s)**.
Pass64 source verification completed, but its standalone packaging stage was not published before Pass65 began. Pass65 is the next packaged checkpoint.
Production source was frozen at the end of that window. The complete `crates/` SHA-256 snapshot captured at freeze is byte-identical to the post-verification snapshot.

## Problem

Pass63 established one typed multi-family inventory and global retained-byte budget boundary, but Γ-QCN quotient factors remained manual-only physical state. This left three connected lifecycle defects:

1. repeated prepared Γ-QCN execution could repeatedly canonicalize the same endpoint rows even when the factor build would amortize cleanly;
2. the multiway cost model continued to charge quotient canonicalization work even when a compatible maintained factor already existed;
3. explicit manual materialization had no ownership-transfer rule if the same factor had first been created by an advisor.

Pass63 also exposed a budget edge case: replacing an incompatible manually owned physical artifact could count both the old fixed bytes and the replacement bytes, falsely rejecting an exact-fit rebuild.

## Hypothesis

A Γ-QCN endpoint factor has a narrow exact read-amortization law:

```text
no factor:  canonicalize endpoint rows on each selected Γ-QCN execution
factor:     canonicalize once at build, then reuse PhysicalRowId -> CanonicalEqKey
```

This is sufficient for a conservative production advisor for plans whose **current** cost model already selects Γ-QCN. It is not sufficient for a complete workload optimizer: write-rate maintenance cost and counterfactual “build this artifact so a different path becomes optimal” planning remain separate frontier work.

## Implementation

### 1. Γ-QCN factor advisor

`PhysicalStore::advise_semantic_quotient_factors` accepts prepared-plan workload samples and the Pass63 shared physical byte policy.

For each currently selected Γ-QCN plan it:

- collects exact unique quotient endpoint bindings;
- measures read-side canonicalization work from physical row counts;
- rejects one-shot builds that do not amortize;
- reuses compatible manual factors without adopting ownership;
- retains compatible advisor-owned factors;
- builds/rebuilds selected factors once and publishes them under one physical epoch;
- evicts only advisor-owned factors that are no longer selected;
- respects independent managed and global estimated-byte ceilings.

A selected factor state constructed for exact byte sizing is carried into publication rather than canonicalized a second time.

### 2. Planner consumes maintained-factor economics

`estimated_quotient_build_work` now checks exact factor compatibility under pinned Γ and current physical row cardinality. A compatible maintained endpoint factor contributes zero quotient canonicalization build work; stale/missing factors retain the old row-count charge.

The same `preferred_nway_order_preserving_join_inputs` cost decision is shared by execution and the factor advisor so the advisor does not claim savings for a Γ-QCN path that the current executor would not use.

### 3. Manual ownership is explicit

`PreparedPlan::materialize_semantic_quotient_factors` now transfers the requested factors out of advisor ownership even when their physical state is already compatible and requires no rebuild. Explicit materialization therefore pins the reconstructible factor until ordinary relation/layout invalidation; a later empty advisor cycle cannot silently evict it.

### 4. Replacement-credit correction

Global byte admission now credits the bytes of an incompatible manually owned artifact when the selected candidate replaces that exact artifact. The correction applies to both semantic-index advice and Γ-QCN-factor advice.

Without the credit, an exact-fit stale rebuild could be rejected because old fixed bytes and new replacement bytes were simultaneously counted.

## Hostile falsification

New hostile coverage verifies:

1. a repeated Γ-QCN workload creates exactly three endpoint factors;
2. execution before advice uses prepared Γ-QCN with zero maintained-key hits, while execution after advice obtains **140 maintained quotient-key hits** and remains equal to the logical result;
3. the planner's quotient-build work becomes zero after compatible factors exist;
4. a repeated no-change advisor cycle retains factors without advancing the physical epoch;
5. an empty workload evicts advisor-owned factors only;
6. explicit manual materialization transfers ownership and survives subsequent advisor eviction;
7. a one-shot workload is rejected as unprofitable;
8. global retained-byte budget can reject otherwise profitable factor builds;
9. a plan for which the current cost model chooses the contiguous path does not acquire unused Γ-QCN factors even with a large execution-count signal;
10. an incompatible manually installed semantic index can be rebuilt under an exact global byte budget that counts replacement rather than old+new simultaneously;
11. the complete `kernel-plan` suite passes at both one and sixteen test threads;
12. the pre-existing non-contiguous/four-way Γ-QCN fixtures remain intact. During source work an accidental fixture edit was detected by the full test suite, compared against pristine Pass63, reverted, and the complete suite rerun green before freeze.

## Boundaries / non-claims

Pass64 deliberately does **not** claim a complete autonomous multi-family optimizer.

- The factor advisor only considers prepared plans whose current cost model already selects Γ-QCN. It does not yet model counterfactual path-shaping where building factors would itself make Γ-QCN preferable.
- `expected_executions` is a read-amortization signal. Write-rate / delta-maintenance cost is not yet modeled.
- Γ-QCN support, specialized I64 indexes, retained statistics, algebraic/future layouts still lack matching family-specific lifecycle laws.
- Pass63 retained-byte estimates remain deterministic planning estimates, not exact allocator/RSS truth.

These limitations belong to the existing multi-family lifecycle/telemetry OPEN; they are not hidden as CLOSED.

## Result

### CLOSED exactly in Pass64 — 4

1. **Γ-QCN endpoint factors were manual-only despite having a clean repeated-read amortization law.** They now have production create/rebuild/retain/reuse/evict lifecycle under shared byte budgets for currently selected Γ-QCN workloads.
2. **Multiway costing ignored maintained quotient-factor reuse.** Compatible maintained factors now remove their endpoint canonicalization work from the Γ-QCN cost estimate.
3. **Explicit manual factor materialization did not pin an already advisor-owned compatible factor.** Manual materialization now transfers lifecycle ownership explicitly.
4. **Stale manual artifact replacement could be double-charged against the global byte budget.** Replacement credit now prevents old+new double counting for semantic-index and Γ-QCN-factor rebuild admission.

These are production lifecycle/costing defects. They do **not** close the historical complete multi-family lifecycle item.

### Historical OPEN accounting

Pass63 authoritative active OPEN count: **24**.

- Historical OPEN fully closed this pass: **0 / 24**.
- Historical OPEN remaining: **24**.
- Genuinely new OPEN created: **0**.

### Advanced but still OPEN

- historical #3 advances again: Γ-QCN endpoint factors join semantic indexes as a second family with measured create/retain/evict behavior, but specialized I64/statistics/support/layout families and cross-family path-shaping remain open;
- historical #4 remains OPEN: there is still no online read/write telemetry, decay/hysteresis or maintenance-rate model;
- historical #5 remains OPEN: Pass63 deterministic retained-byte accounting is used by the new advisor, but allocator/RSS/pressure integration is unchanged;
- historical #2 remains OPEN: bounded 3–8-leaf Γ-QCN planning remains the execution envelope.

### Active OPEN after Pass64 — 24

1. ⬜ Structural/custom-equivalence physical indexing is still incomplete: durable recursive-key encoding/versioning, persisted structural indexes, arbitrary/plugin canonical laws and structural ordering remain open.
2. ⬜ General nested/multiway/bushy Join planning beyond the verified bounded 3–8-leaf Γ-QCN fragment: adaptive/unbounded search, richer predicate graphs and fully general indexed/typed subset execution.
3. ⬜ Complete multi-family physical lifecycle: Pass64 adds Γ-QCN factor advice, but specialized I64 indexes, statistics, Γ-QCN support, algebraic/future layouts, counterfactual path-shaping and write-maintenance costing remain open.
4. ⬜ Autonomous workload statistics/telemetry, histograms/correlation, read/write rates, decay/hysteresis and lifecycle scheduling.
5. ⬜ Exact allocator/resident-memory accounting, external memory-pressure integration and rebuild scheduling.
6. ⬜ Canonical-key/cache encoding-version migration and rebuild/compatibility law for long-lived physical artifacts.
7. ⬜ Remaining physical layouts beyond the algebraic family, plus explicit `OrderedView`/pagination.
8. ⬜ Secondary-index / alternate-layout rebuild economics and fast reconstruction after recovery.
9. ⬜ Persistent outer artifact-map metadata: catalogs remain ordinary `BTreeMap<K, Arc<State>>` with O(number of artifacts) candidate clone metadata.
10. ⬜ Durable revision DAG / branch+merge ancestry and merge replay.
11. ⬜ General historical durable-format migration framework.
12. ⬜ Arbitrary/plugin semantic executable artifact packaging/signing/authentication/deployment.
13. ⬜ Transaction intent/outcome retention + GC; Program7's compact exact-intent candidate remains rejected pending a stronger exact content-identity design.
14. ⬜ Streaming/chunked checkpoints and metadata.
15. ⬜ Real machine power-loss assurance plus Windows/network-FS/FUSE durability semantics.
16. ⬜ General lock-poison/restart policy.
17. ⬜ Group commit / async durability.
18. ⬜ Replication / consensus and broader distribution architecture.
19. ⬜ Durable-store authentication/MAC; CRC32C remains corruption detection only.
20. ⬜ Formal power-loss proof for rename/fsync/GC protocol.
21. ⬜ Transaction repair runtime.
22. ⬜ Formal mechanization of the remaining semantic/transport/retention/power-loss obligations.
23. ⬜ Maintained I64 Group constant-factor gap.
24. ⬜ Maintained I64 TopK constant-factor gap.

## Verification

Rust: `rustc 1.98.1 (48a229cea 2026-09-01)`.

Final frozen bytes PASS:

- `cargo fmt --all -- --check`;
- `cargo check --workspace --all-targets`;
- `cargo test --workspace --all-targets`;
- `cargo clippy --workspace --all-targets -- -D warnings`;
- `cargo test --workspace --all-targets --release`;
- `cargo build --workspace --release`;
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps`;
- `RUSTFLAGS='-C overflow-checks=yes' cargo test --workspace --all-targets --release`.

The first overflow-check workspace invocation hit the external cold-compilation timeout and was not counted. The warmed retry completed successfully.

Static snapshot at the completed Pass64 source gate:

- **410 declared tests**;
- **159 `kernel-plan` tests**;
- **21 crates**;
- **57,089 Rust LOC**;
- **0 external registry/git Cargo sources**;
- **0 `unsafe` hits**;
- **19 existing `#[allow(...)]`**, no new suppression;
- **0 TODO/FIXME/todo!/unimplemented! hits**.

The complete `crates/` SHA-256 list captured at source freeze exactly matches the post-gate list.

## Next frontier

The clean next main-branch step is to stop adding isolated artifact policies and move toward **shared semantic projection state**: one revision/relation/column/equivalence derivative `PhysicalRowId -> CanonicalEqKey` that semantic indexes, Γ-QCN factors, statistics and maintained Group/Join can consume without separately canonicalizing or storing equivalent projection state.

That would give the multi-family advisor one actual shared object whose memory, maintenance and reuse benefit can be priced across families. The alternative recovery/rebuild-economics branch remains valid, especially once R&D Program5/7 results return.
