# CFMD Pass46 Report — Retained Γ-Bound Key Statistics / Measured Transient Costing

**Status:** VERIFIED on Rust 1.98.1.

**Production source window:** 2026-09-20 15:38:33 → 15:58:25 +03:00 (**19m52s**). Production source was frozen at 15:58:25; the final verification gate found no correctness defect, so the freeze was not lifted.

**Scope:** physical planner/statistics layer only. Pass46 does not change semantic `Revision=(S,Γ,M)`, transaction authority, durable authority, or exact query semantics.

## 1. Problem

Pass45 deliberately refused to credit unknown transient-index distinctness in multiway planning. That was correct, but left two connected gaps:

1. there was no retained reconstructible Γ-bound key-cardinality state independent of a full persisted index;
2. without measured distinctness, the planner either had to ignore a potentially useful transient access in multiway costing or let direct execution build a speculative transient index and discover only afterwards that duplicate-heavy data made it unprofitable.

A naïve attempt to move directly to arbitrary leaf permutation was also hostile-reviewed and rejected for this pass: the current physical Join API has deterministic left-major/right-minor row production, so arbitrary leaf permutation would change observable physical row order unless an explicit order-restoration/provenance mechanism is added. Pass46 therefore implements the prerequisite statistics layer instead of claiming an unsound general-bushy closure.

## 2. Retained semantic key statistics

Pass46 adds reconstructible `MaterializedSemanticStatisticsState` keyed by the existing `SemanticIndexBinding`.

The state retains exact key multiplicities under the same primitive canonical Γ law used by persisted semantic indexes:

```text
Vec<CanonicalEqKey> -> multiplicity
```

Public planner-facing summary:

```text
SemanticKeyStatistics {
    row_count,
    distinct_key_count,
}
```

Properties:

- single-column and composite/mixed primitive keys are supported;
- binding resolution is shared with `MaterializedSemanticIndexState`, so index and statistics cannot silently drift onto different canonicalization contracts;
- unsupported structural/custom equivalences still use exact fallback rather than invented keys;
- statistics are reconstructible physical state, never semantic authority;
- inconsistent row-count metadata is ignored by planner access rather than trusted;
- relation reinstall invalidates bound statistics;
- Γ drift behind the same `SemanticId` makes the state incompatible and relation mutation refuses to maintain it under the changed law without rebuild.

## 3. Atomic maintenance and runtime publication

Statistics participate in the same physical relation transition as indexes.

Before mutation:

- Γ compatibility is checked;
- the complete physical delta is validated against statistics state;
- removal multiplicity cannot exceed the retained canonical bucket multiplicity.

During the prepared physical transition, relation/index/statistics changes are applied together. Distinct-key birth/death and duplicate multiplicity changes remain exact.

`RuntimeRevisionCell::install_semantic_statistics` publishes the derived state through one immutable root swap. Existing readers keep the prior snapshot, new readers observe the incremented physical root version under the same semantic revision, and an already-prepared runtime transition becomes stale. No statistics bytes become durable semantic authority.

## 4. Measured transient Join costing

`right_scan_join_access_decision` now consumes compatible retained statistics when present.

For transient I64 or generic semantic candidates, the planner can use measured `distinct_key_count` rather than the optimistic `distinct = row_count` assumption.

Consequences:

- duplicate-heavy data can choose `FullScan` before building a transient index;
- selective measured data can safely admit a transient candidate;
- the contiguous multiway planner may credit transient access only when retained statistics exist;
- without retained statistics, the conservative Pass45 multiway rule remains unchanged;
- execution still performs the Pass45 post-build actual-distinct recheck, so retained statistics remain advisory rather than authority.

## 5. Hostile / regression evidence

Pass46 verifies:

- exact distinct-key birth/death under sequential relation deltas;
- duplicate multiplicity handling;
- relation reinstall invalidates statistics;
- Γ change invalidates statistics and rejects mutation-through-stale-statistics;
- runtime publication preserves immutable reader snapshots and invalidates an old prepared transition;
- deliberately corrupted row-count statistics are ignored rather than trusted;
- duplicate-heavy Join avoids a transient build entirely when retained statistics show it cannot amortize;
- multiway costing admits transient access when measured statistics make it profitable;
- composite `(TextAsciiCaseInsensitive, I64Exact)` statistics produce the same distinct-key count as the corresponding persisted semantic index;
- debug and release `kernel-plan` suites agree;
- no new Clippy suppression was added; Pass45 and Pass46 both contain 14 existing `#[allow(...)]` sites;
- no side-effecting mutation is hidden in a `debug_assert!` pattern.

Freeze result: **118 passed, 0 failed, 1 ignored diagnostic benchmark** in `kernel-plan`.

## 6. Verification

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

- 347 declared Rust tests;
- 119 `kernel-plan` tests (118 normal + 1 ignored diagnostic benchmark);
- 21 crates;
- 46,869 Rust LOC under `crates/`;
- 0 external Cargo registry/git sources;
- 0 `unsafe` in `crates/`;
- 0 TODO/FIXME/todo!/unimplemented! markers in `crates/`.

Evidence is retained under `evidence/pass46/` inside the workspace.

## 7. Problem ledger

### CLOSED exactly in Pass46 — 2

1. ✅ **No retained Γ-bound exact key-cardinality statistics existed independently of a full persisted index.** Pass46 adds reconstructible single/composite primitive key multiplicity state, shared canonical binding resolution, exact atomic delta maintenance, Γ invalidation and immutable runtime publication.
2. ✅ **Transient Join costing could not use measured selectivity without already having a persisted index.** Direct and multiway right-side access decisions now consume retained exact distinctness when available, allowing profitable transient access to be credited and duplicate-heavy transient builds to be rejected before construction.

### Historical OPEN from Pass45 fully closed this pass

**0 / 22.** Pass46 materially advances the statistics prerequisite, but the broad historical items remain wider than exact retained key cardinality.

### Advanced but still OPEN

1. 🟨 **General nested/multiway/bushy Join planning.** Pass44–46 provide leaf-order-preserving interval reassociation, one multi-family access decision, and measured retained key cardinality. Arbitrary leaf permutation/non-contiguous general bushy plans remain OPEN because the current physical row-order contract needs explicit order restoration/provenance before permutation is safe.
2. 🟨 **Statistics ecosystem.** Exact Γ-bound row/distinct-key statistics now exist for current primitive single/composite keys. Histograms, cross-column/cross-relation correlation, autonomous telemetry, decay/hysteresis, lifecycle eviction and byte-accurate footprint policy remain OPEN.
3. 🟨 **Multi-family lifecycle advisor.** Generic semantic-index lifecycle remains Pass43-owned; specialized I64, retained statistics and future layouts do not yet share one create/retain/evict/memory-benefit contract.
4. 🟨 **I64 Group/TopK constant factors.** Existing performance debt is unchanged by Pass46.

### Historical / active OPEN after Pass46 — 22

1. ⬜ Structural/custom-equivalence canonical indexing and typed production.
2. ⬜ General nested/multiway/bushy Join planning: arbitrary leaf permutation/non-contiguous plans/richer predicate graphs.
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

### New OPEN created in Pass46

**0.** The leaf-permutation/order-restoration issue refines the already-open general multiway/bushy item; it is not a new independent architectural debt.

## 8. Rejected / deliberately deferred routes

- no arbitrary leaf permutation that silently changes current physical Bag row order;
- no treating statistics as semantic authority;
- no hidden host `Eq`/`Hash`/`Ord` keying instead of pinned Γ canonicalization;
- no optimistic distinctness represented as measured data;
- no automatic statistics lifecycle hidden inside index lifecycle before a common memory/benefit contract exists;
- no new lint suppression.

## 9. Result / next direction

The next clean planner step is to add an explicit physical order-restoration/provenance mechanism for multiway execution and then move interval DP to subset/general-bushy enumeration without changing logical output semantics.

In parallel, retained statistics should grow from exact key cardinality into a lifecycle-owned statistics catalog with histograms/correlation only where justified by workload evidence and a common byte-level budget.
