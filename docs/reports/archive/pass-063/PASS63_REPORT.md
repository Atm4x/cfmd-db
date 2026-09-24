# PASS63 REPORT — multi-family physical inventory and memory-bounded semantic-index lifecycle

Status: **VERIFIED**

Production source window: **2026-09-20 21:36:19 UTC → 21:56:25 UTC** (20m06s). Production source was frozen at the end of that window. All subsequent work was verification, documentation and packaging; the frozen `crates/` SHA-256 snapshot was rechecked after the final gate and remained byte-identical.

## Problem

Pass43 introduced ownership-safe workload-driven lifecycle for the generic semantic-index family, but its budget law was still a semantic-index-local proxy:

```text
max_managed_key_cells
```

That left two related physical-architecture defects:

1. the advisor could reason as if other retained physical families were free even when `PhysicalStore` already owned relation layouts, specialized I64 indexes, retained Γ statistics, Γ-QCN quotient factors/support or shared dense identity backing;
2. ownership metadata itself was special-cased as `advisor_managed_semantic_indexes`, so later physical families had no common artifact identity/inventory boundary to extend.

This did **not** mean that exact allocator/RSS accounting or a complete cross-family benefit model already existed. Those broader historical items remain open.

## Hypothesis

Memory/lifecycle policy should separate three concepts:

```text
semantic benefit/cost       -> workload work units
retained-memory constraint  -> deterministic byte estimate
artifact ownership          -> typed physical-family identity
```

A semantic-index advisor may create/evict only artifacts it owns, but its global memory ceiling must include non-evictable physical state as fixed cost. Shared backing must be counted once rather than once per consumer.

The retained-byte quantity must be deterministic and overflow-safe. It is a planning estimate, **not** an invented claim of exact RSS or allocator heap usage.

## Implementation

### 1. Typed multi-family physical inventory

`PhysicalStore::artifact_memory_report()` now reports retained artifacts by `PhysicalArtifactFamily`:

- `RelationLayout`;
- `SharedDenseIdentityMap`;
- `I64Index`;
- `SemanticIndex`;
- `SemanticQuotientFactor`;
- `SemanticQuotientSupport`;
- `SemanticStatistics`.

Each family reports:

```text
artifacts
advisor_managed_artifacts
estimated_retained_bytes
```

and the store exposes one saturating `total_estimated_retained_bytes`.

The report covers current row/column/typed/algebraic layouts, stable-row metadata, specialized and generic indexes, Γ-QCN factor/support state and retained semantic statistics.

### 2. Shared backing is not double-counted

`DenseLiveEntityIds` columns may share one `Arc<DenseEntityIds>`. The relation-column payloads themselves remain part of their relation layouts, while the shared external↔local dense identity map is inventoried separately and deduplicated by in-process `Arc` identity.

A hostile fixture installs two dense-reference columns that share the same map and verifies:

```text
RelationLayout artifacts       = 1
SharedDenseIdentityMap artifacts = 1
```

rather than charging the shared map twice or ignoring it entirely.

### 3. Deterministic retained-byte estimators

Pass63 adds bounded retained-size estimates for the physical structures needed by the current inventory, including:

- `DenseEntityIds`;
- `SemanticBucketIndex` including bucket/reverse owned payloads;
- installed native relations and stable-row metadata;
- recursive `AlgebraicNativeColumn` storage;
- I64 indexes;
- semantic indexes;
- Γ-QCN quotient factors;
- retained semantic statistics;
- Γ-QCN support state.

All newly introduced size aggregation uses saturating arithmetic so an extreme state cannot wrap in release and turn a memory ceiling into a fail-open decision.

Allocator/BTree node overhead and OS RSS are intentionally outside the estimator contract. This is a deterministic planner quantity, not an exact heap profiler.

### 4. Advisor policy now has independent work and memory constraints

`SemanticIndexAdvisorPolicy` now contains:

```text
max_managed_key_cells
max_managed_estimated_bytes
max_total_estimated_bytes
```

`max_managed_key_cells` remains a compatibility/work-size proxy. `max_managed_estimated_bytes` constrains advisor-owned semantic indexes. `max_total_estimated_bytes` additionally includes fixed physical state that this advisor is not allowed to evict.

Profitability still comes from expected query work saved versus build work. A compact but unprofitable index is not built merely because it fits in memory, and a profitable index is rejected if the retained-byte budget cannot admit it.

### 5. Global budget includes other physical families

Before selection, the advisor obtains the complete physical memory inventory. Existing advisor-owned semantic indexes are treated as replaceable cost. Everything else is fixed cost for this policy turn, including relation layouts and other physical families.

A hostile test materializes retained semantic statistics before advising an otherwise profitable semantic index, then places the global byte ceiling only one byte above current fixed state. The index is rejected while the statistics remain intact.

This establishes the key authority rule:

> one advisor cannot satisfy its budget by silently evicting a family whose lifecycle/benefit contract it does not own.

### 6. Generic advisor-owned artifact identity

The former semantic-index-only ownership set is replaced by typed internal `ManagedPhysicalArtifact` identities for:

- specialized I64 indexes;
- semantic indexes;
- Γ-QCN quotient factors;
- Γ-QCN support;
- semantic statistics.

Relation reinstall invalidation can therefore reason through one physical-artifact identity boundary.

**Important boundary:** Pass63 does not pretend that all these families now have autonomous create/retain/evict policies. Only generic semantic indexes currently participate in the workload benefit/selection policy. The other variants establish common ownership/inventory architecture for later lifecycle algorithms.

## Hostile falsification

New/extended hostile coverage verifies:

1. estimated-byte budget can reject an otherwise profitable semantic index;
2. a global byte budget counts retained non-index physical state and cannot evict it;
3. shared dense identity backing is charged exactly once across multiple dense columns;
4. Γ-QCN factor/support/statistics inventory is visible as distinct physical families;
5. semantic bucket retained-size accounting grows with bucket and reverse payloads;
6. existing advisor rule “evict only its own index, never a manually installed compatible index” remains green;
7. strict Clippy required no new suppression.

The entire workspace debug/release/overflow gates remain green.

## Result

### CLOSED exactly in Pass63 — 2

1. **Semantic-index lifecycle had no retained-byte/global physical memory constraint.** The advisor now has independent managed/global estimated-byte ceilings and counts non-evictable physical families as fixed cost rather than assuming they are free.
2. **Physical lifecycle ownership/inventory was semantic-index-special-cased and could not account shared backing coherently.** `PhysicalStore` now has a typed multi-family artifact inventory/ownership boundary with deduplicated shared dense-identity backing.

These are narrower production defects. They do **not** fully close historical multi-family lifecycle or exact byte/RSS accounting.

### Historical OPEN accounting

Pass62 authoritative active OPEN count: **24**.

- Historical OPEN fully closed this pass: **0 / 24**.
- Historical OPEN remaining: **24**.
- Genuinely new OPEN created: **0**.

### Advanced but still OPEN

- historical #3 is substantially advanced: physical families now share typed inventory/ownership identity and one advisor observes global retained memory, but I64/statistics/QCN/layout families still lack their own measured benefit/create/retain/evict/rebuild policies;
- historical #5 is substantially advanced: deterministic retained-byte and shared-backing accounting now exists, but exact allocator/RSS accounting, pressure integration and rebuild scheduling remain open;
- historical #4 remains open: no online telemetry, histograms/correlation, decay/hysteresis or autonomous lifecycle scheduler was added;
- nested algebraic/dense-local representation choice remains a future cross-family lowering decision.

### Active OPEN after Pass63 — 24

1. ⬜ Structural/custom-equivalence physical indexing is still incomplete: durable recursive-key encoding/versioning, persisted structural indexes, arbitrary/plugin canonical laws and structural ordering remain open.
2. ⬜ General nested/multiway/bushy Join planning beyond the verified bounded 3–8-leaf Γ-QCN fragment: adaptive/unbounded search, richer predicate graphs and fully general indexed/typed subset execution.
3. ⬜ Complete multi-family physical lifecycle: specialized I64 indexes, statistics, Γ-QCN factors/support, algebraic/future layouts still need family-specific benefit/create/retain/share/evict/rebuild policy on top of the Pass63 common inventory/budget boundary.
4. ⬜ Autonomous workload statistics/telemetry, histograms/correlation, decay/hysteresis and lifecycle scheduling.
5. ⬜ Exact allocator/resident-memory accounting, external memory-pressure integration and rebuild scheduling. Pass63 deterministic retained-byte/shared-backing accounting is planning-grade rather than RSS truth.
6. ⬜ Canonical-key/cache encoding-version migration and rebuild/compatibility law for long-lived physical artifacts.
7. ⬜ Remaining physical layouts beyond the algebraic family, plus explicit `OrderedView`/pagination.
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
23. ⬜ Maintained I64 Group constant-factor gap.
24. ⬜ Maintained I64 TopK constant-factor gap.

## Post-freeze R&D Program7 hostile review

The external Program7 delta-native durability closeout was reviewed after the Pass63 production freeze. Its patch was **not integrated**. A concrete hostile counterexample shows that `RelationDataExact { source/target RevisionId, delta, Γ deployment }` is insufficient to preserve the existing exact retry contract when the caller can supply an independent `target_revision`: after later heads + checkpoint/compaction, the same txid/IDs/delta with different untouched target content can compare equal before `prepare_revision()` validates target content.

The Program7 space diagnosis is accepted, but production status remains OPEN. See `PASS63_RND_PROGRAM7_REVIEW.md` and `evidence/pass63/PROGRAM7_HOSTILE_EXACT_RETRY_COUNTEREXAMPLE.txt`. No production bytes changed during this review, so the Pass63 source freeze and final Rust gate remain valid.

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

Cold release and overflow-check invocations that hit the external compilation timeout were not counted. Warmed retries completed successfully.

Static snapshot before packaging:

- **405 declared tests**;
- **154 `kernel-plan` tests**;
- **21 crates**;
- **56,341 Rust LOC**;
- **0 external registry/git Cargo sources**;
- **0 `unsafe` hits**;
- **19 existing `#[allow(...)]`**, no new suppression;
- **0 TODO/FIXME/todo!/unimplemented! hits**.

The complete `crates/` SHA-256 list captured at source freeze exactly matches the post-gate list.

## Next frontier

The main branch should not immediately invent autonomous policies for every physical family without measured cost/benefit laws. The clean next production cluster is either:

1. shared semantic projection/canonical-key derivative reuse across semantic index + QCN + statistics/Group/Join, which gives several current families a common measurable maintenance object; or
2. recovery/rebuild economics using the new byte inventory, so lifecycle policy can price rebuild versus retained memory rather than only execution cost.

External R&D Program7 (delta-native durability) is deliberately not integrated or assumed here; it should be hostile-reviewed when its closeout arrives.
