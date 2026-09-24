# PASS40 REPORT — persisted primitive semantic indexes + physical planner consumption + I64 Group hot-path reduction

Status: **VERIFIED** on Rust **1.98.1**.

Primary Pass40 source wall-clock: **2026-09-20 01:59:53 UTC → 02:19:55 UTC (~20m02s)**. After that cycle the user explicitly requested one final architecture cleanup before packaging; source was reopened only to unify the duplicate semantic bucket engine, then the complete Rust gate was rerun from the modified state. The packaged checkpoint therefore includes that post-window requested cleanup and is verified from its final source.

Pass40 continues the semantic-index integration from Pass39 into the physical storage/planner layer. CLOSED items below are actual eliminated architecture/functional problems. Individual tests, hostile cases and stress loops are evidence only and are not counted as separate CLOSED problems.

A separate hostile review of the proposed first-class `Chain<T>` / `Lineage<T>` abstraction was also performed. It was **not integrated** into the universal kernel.

## 1. Problem

Pass39 proved and integrated exact Γ-bound canonical keys into maintained Join/Group/TopK state, but one historical Pass26 data-plane problem still remained at the physical layer:

```text
PhysicalStore persisted exact semantic index
    existed only as the specialized I64 path

Text / F64Bitwise / Bool / entity exact equality
    had no equivalent long-lived physical row-handle index

physical Filter / Join executor
    could not consume a non-I64 persisted semantic index
```

At the same time, the maintained I64 Group benchmark still showed a large constant-factor gap against a hand-written count-map update.

## 2. Design

### 2.1 Persisted primitive semantic index in `PhysicalStore`

Pass40 adds:

```text
SemanticIndexBinding
    relation
    physical layout
    key column
    equivalence SemanticId

MaterializedSemanticIndexState
    exact ResolvedPrimitiveEquivalence
    shared SemanticBucketIndex<CanonicalEqKey, PhysicalRowId>
```

The index deliberately stores stable physical row identities, not copied logical rows. It is reconstructible physical state and is not semantic or durable authority.

The index can be built for every currently canonicalizable builtin primitive equality contract:

- UnitExact;
- BoolExact;
- I64Exact;
- F64Bitwise;
- TextExact;
- TextAsciiCaseInsensitive;
- LiveEntityIdExact(entity_type);
- HistoricalEntityIdExact(entity_type).

The existing specialized I64 index remains intact. The generic semantic index extends coverage; it does not replace or slow the proven I64 specialization.

### 2.2 Atomic physical maintenance

`PhysicalStore::apply_relation_delta*` now prevalidates and updates all semantic indexes bound to the mutated relation/layout in the same candidate transition as the relation itself and the existing I64 indexes.

The index therefore cannot become a separately published second state.

Relation reinstall invalidates semantic indexes bound to that exact relation/layout, matching the existing reconstructible-index authority rule.

### 2.3 Γ compatibility

`MaterializedSemanticIndexState` stores the resolved primitive equivalence used to build it. Before maintenance or fast-path execution, that contract is re-resolved from the pinned `SemanticContext + SemanticRegistry`.

If Γ changes under the same nominal semantic ID:

- physical semantic-index maintenance returns `SemanticContextTransitionRequiresRebuild`;
- query execution does **not** silently use the stale index and falls back to the exact semantic path.

The final hostile regression explicitly changes Text ASCII-CI to TextExact under the same SemanticId and verifies that a stale ASCII-CI index is not consumed by an exact-text Filter.

### 2.4 Physical Filter/Join consumption

The former `IndexedI64IfAvailable` planning label is generalized to:

```text
IndexedPrimitiveIfAvailable
```

For direct physical scans, execution now attempts a compatible persisted semantic index for:

- `FilterEqConst(Scan(...))`;
- direct equality `Join(Scan(...), Scan(...))`;
- projected variants routed through the existing physical project path.

Candidate rows obtained from the index are still checked with the exact resolved equivalence before output. The index is an accelerator, never an equality oracle independent of Γ.

If no compatible persisted semantic index exists, the existing specialized I64 path and then exact fallback remain available.


### 2.5 Shared semantic-index engine

The final architecture cleanup removes the duplicate physical bucket implementation.
`kernel-plan::MaterializedSemanticIndexState` now uses the same generic
`kernel-semantic-index::SemanticBucketIndex<Key, Identity>` engine as maintained Join/TopK.
The engine itself was refined so bucket membership preserves identity insertion order while the reverse map still guarantees one live key per identity. This matters for physical row handles: a `BTreeSet`-ordered bucket could reorder reused handles relative to logical insertion/tie order.

The result is one bucket/reverse-map implementation for maintained derivative identities and physical stable row identities, with the semantic binding still pinned by revision + module digest + key-encoding revision. Specialized I64 storage indexes remain intentionally separate because they are a distinct optimized representation, not a duplicate generic semantic bucket engine.

## 3. Maintained I64 Group — partial optimization, not closure

Pass40 also attacks the historical I64 Group constant-factor gap.

Two costs were removed from the common tiny-delta path:

1. a state already built and pinned to the exact `SemanticContext` no longer re-admits the same semantic modules for every I64 count delta; it performs the required row-shape validation and uses the already admitted I64 fast path;
2. the common one-remove/one-insert replacement no longer allocates the generic changes-plan. When a singleton group dies while a new key is born, the existing group slot can be reused directly.

Seven post-unification release process runs at 50k groups:

```text
run1  365 ns / 155 ns = 2.354x
run2  365 ns / 152 ns = 2.401x
run3  379 ns / 162 ns = 2.339x
run4  381 ns / 161 ns = 2.366x
run5  361 ns / 150 ns = 2.406x
run6  370 ns / 156 ns = 2.371x
run7  365 ns / 159 ns = 2.295x
```

Historical pre-Pass40 runs of the same benchmark family were around **4.8x** the hand-written baseline. Pass40 materially reduces the gap to a stable roughly **2.3–2.4x**, but **does not close it**.

Raw evidence: `evidence/pass40/FINAL_I64_GROUP_BENCH_7RUN.log`.

## 4. Hostile falsification

Pass40 verifies or retains coverage for:

- physical semantic-index construction over the mixed primitive carrier set;
- Text ASCII-CI duplicate semantic buckets;
- direct Filter and Join consumption of the persisted Text semantic index;
- Bag multiplicity preservation;
- exact row-handle lookup rather than copied-row authority;
- atomic maintenance alongside relation delta;
- relation reinstall invalidation;
- Γ mismatch requiring rebuild for maintenance;
- stale Γ-bound semantic index being skipped by physical execution;
- release/overflow behavior;
- static audit for mutating side effects hidden in `debug_assert!` after the Pass39 release-only incident.

The persisted semantic-index regressions were additionally repeated hundreds of times during the wall-clock attack; no intermittent failure appeared. These repetitions are evidence, not separate CLOSED problems.

## 5. `Chain<T>` / `Lineage<T>` hostile design review

Result: **REJECTED as a new universal logical primitive.**

The proposed rooted single-parent acyclic persistent structure is already representable as:

```text
parent : PartialMap<Node, Node>
+ payload relation/map
+ rooted/acyclic/reachability constraints
+ ancestry/path derived as Seq<Node>
```

Single-parent + rooted acyclicity gives the unique root→node path. Append/branch are typed rewrites; ancestor/LCA/path are recursive derived queries. Revision history already provides persistent semantic versions.

The valuable part of the idea is retained as a possible **surface refinement/constraint bundle** plus physical specialization. Appropriate reconstructible lowerings include the already-existing `AdjacencyList` family, depth/jump tables, binary lifting, persistent chunks/ropes and shared path caches. Path compression must not rewrite the authoritative parent relation; shortcut edges may only be derivative physical state.

Adding a universal kernel primitive would impose type/change/query/transport/durability/merge/lens/planner/formal obligations without adding current expressive power.

Full review: `PASS40_CHAIN_LINEAGE_HOSTILE_REVIEW.md`.

## 6. Verification

Final Rust 1.98.1 gate after the final regression:

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

Metrics:

```text
311 declared tests
83 kernel-plan tests
69 kernel-query tests
30 kernel-semantics tests
4 kernel-semantic-index tests
21 crates
40,331 Rust LOC
0 external Cargo sources
0 unsafe
```

The existing specialized persisted-I64 reference benchmark still behaves as expected in this workspace: 20k-row persisted join ~3.10 ms vs ~2.69 ms hand prebuilt baseline and ~5.81 ms ephemeral rebuild. This is reference evidence only; the I64 persisted-index problem was already addressed by earlier passes.

## 7. Problem ledger

### CLOSED exactly in Pass40

1. ✅ **`PhysicalStore` lacked a persisted/reconstructible exact semantic index for current builtin primitive equality domains beyond the specialized I64 path, and the physical Filter/Join executor could not consume such non-I64 indexes.** Pass40 adds Γ-bound `MaterializedSemanticIndexState`, atomic relation/index maintenance, rebuild/invalidation rules, and direct persisted semantic-index consumption by primitive Filter/Join execution. The historical Pass26 “persisted Text/F64/Bool/entity indexes + physical planner integration” problem is therefore closed at the same direct-plan level as the existing I64 index. Automatic cost-based index creation/selection remains a separate optimization problem.
2. ✅ **Physical and maintained semantic indexes had started to duplicate the same bucket/reverse-map machinery.** The final Pass40 cleanup makes `kernel-semantic-index::SemanticBucketIndex<Key, Identity>` the shared implementation for both derivative row identities and `PhysicalRowId`, while preserving insertion order inside equal-key buckets. The duplicate physical `BTreeMap<CanonicalEqKey, Vec<PhysicalRowId>>` engine is removed.

### Partially advanced, still OPEN

1. 🟨 **Maintained I64 Group constant-factor gap.** Improved from roughly 4.8x historical hand baseline to roughly 2.3–2.4x, but a meaningful gap remains.
2. 🟨 **Physical semantic-index planning policy.** Installed compatible primitive indexes are now consumed correctly, but there is no cost/selectivity model deciding when an index should be created, retained or preferred. Small-relation constant factors therefore remain planner debt.
3. 🟨 **Physical semantic-index lifecycle.** Indexes are reconstructible and revision/Γ-safe, but automatic rebuild scheduling, shared-index reuse, memory budgeting and eviction are not implemented.

### Historical Pass26 OPEN backlog still active

1. ⬜ I64 TopK constant-factor gap.
2. ⬜ Group/TopK as typed-batch producers.
3. ⬜ Maintained I64 Group constant-factor gap — materially improved in Pass40, still OPEN.
4. ⬜ Structural/custom-equivalence Group canonical indexing. Primitive/Text portion remains closed from Pass39.
5. ✅ Persisted Text/F64/Bool/entity exact primitive indexes in `PhysicalStore` + direct physical Filter/Join consumption — **CLOSED in Pass40**. Cost/selectivity/automatic-index policy is tracked separately and remains OPEN.
6. ⬜ Nested/multiway/mixed-key joins and broader join planning.
7. ⬜ Remaining physical layouts + OrderedView/pagination.
8. ⬜ Remaining durability/history assurance: durable DAG/merge ancestry, universal format migration, arbitrary semantic artifacts, power-loss/cross-filesystem assurance, scalability/distributed/authenticated durability.
9. ⬜ Transaction repair runtime, distribution and formal mechanization.

### Additional active OPEN after Pass40

1. ⬜ Structural/custom canonical-key laws for persisted and maintained indexes.
2. ⬜ Cost-based index selection / small-relation crossover policy.
3. ⬜ Shared semantic-index reuse across multiple plans/operators.
4. ⬜ Physical-index memory budgeting, eviction and automatic rebuild scheduling.
5. ⬜ Canonical key encoding migration/version compatibility for long-lived physical caches.
6. ⬜ I64 TopK and residual I64 Group constant-factor tuning.
7. ⬜ Typed-batch Group/TopK output paths.
8. ⬜ Multi-column/mixed-key persisted physical semantic indexes and multiway join planning.

## 8. Result / next direction

Pass40 closes the remaining primitive persisted semantic-index/planner hole without expanding semantic authority and without forcing a workload-specific `Lineage` primitive into the universal layer.

The next high-value data-plane work is no longer “make Text/F64/entity physical equality indexable”; that part exists. The clean next choices are:

1. attack typed-batch Group/TopK and the remaining I64 constant factors; and/or
2. extend join planning to multi-column/mixed-key/multiway plans with cost-aware persisted-index selection.

The `Chain/Lineage` idea should remain a standard-library refinement/physical-hint candidate, not a kernel primitive.
