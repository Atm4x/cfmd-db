# PASS78 REPORT — semantic resource accounting + Γ Support-Atom Materialization Fabric foundation

Status: **VERIFIED**.

Source freeze: **2026-09-21 15:44:11 UTC**.

Production diff relative to Pass77:

- `crates/kernel-semantics/src/lib.rs`
- `crates/kernel-semantics/src/observable.rs`
- `crates/kernel-semantics/src/support_atom.rs` (new)
- `crates/kernel-plan/src/lib.rs`
- `crates/kernel-plan/src/algebraic_native.rs`

Pass78 has two deliberately orthogonal production layers: size-aware semantic resource accounting and Stage A/B integration of the Γ Support-Atom Materialization Fabric (Γ-SAMF). Existing artifact families remain in place as parity oracles; no destructive advisor/durability migration is claimed.

## Problem → hypothesis → implementation → falsification → result

### 1. Recovery work accounting was size-blind

**Problem.** Pass75/76 charged one key-cell evaluation equally for an I64, a one-byte Text and a recursively large structural value.

**Hypothesis.** Recovery admission needs a deterministic representation-independent structural input metric in addition to key-cell count, explicitly weaker than CPU/RSS truth.

**Implementation.** `kernel-semantics` defines `SemanticWorkEstimate { value_nodes, payload_bytes }`. Logical `Value` and `CanonicalEqKey` have the same deterministic traversal law. Row/value/I64/typed-algebraic storage can compute the metric without reconstructing full logical rows. `PhysicalRecoveryPolicy` adds `max_advisor_rebuild_semantic_work_units`; advisor recovery ranks by semantic work before key-cell count and caches per-column estimates during one scheduling pass.

**Falsification.** Two two-row Text indexes have the same key-evaluation count but different payload sizes. A semantic-work budget admits the short index and defers the long one. A typed algebraic `Seq<Text>` produces the same work estimate as its logical `Value` representation.

**Result.** Recovery no longer equates all semantic cells by count, while manual durable intent remains non-budget-evictable.

### 2. Physical semantic state duplicated the same relation partition across artifact families

**Problem.** Semantic index fibers, quotient fibers and statistics independently encode overlapping maps from Γ-semantic classes to `PhysicalRowId`s/counts. The Γ-SAMF R&D showed that these are views over one joint finite partition induced by active observables.

**Hypothesis.** Introduce the common support-atom substrate first, retain legacy states as exact parity oracles, and postpone destructive advisor/durable migration.

**Implementation.** `kernel-semantics::support_atom::SupportAtomFabric<RowId>` partitions finite support by one revision-local product observable. The **atom identity is the product observable's existing `EqClassId`**, not a new atom namespace. It maintains:

- atom class → row fiber;
- row → atom class;
- product signature → atom class;
- per-coordinate inverse projection from a class to participating atom classes.

`kernel-plan::MaterializedObservableAtomState` binds this fabric to a real relation/layout and `SemanticIndexBinding`, constructs observables through the pinned `RevisionObservableCatalog`, keeps a certified product-projection morphism, and exposes exact joint/projected probes and mass/count views over `PhysicalRowId`.

The state is reconstructible physical data only. `RevisionObservableId`/`EqClassId` remain catalog-realization-local and are not durable semantic authority.

**Falsification.** Production hostiles prove:

- multi-column joint atom fibers and projected fibers are exact;
- SAMF probes/counts match legacy semantic-index/statistics views;
- relation deltas maintain exact fibers incrementally;
- Γ drift rejects before relation/fabric mutation;
- physical layout replacement invalidates the fabric;
- publication uses a new immutable runtime physical root without changing logical Revision;
- foreign catalog classes from the same semantic revision are rejected;
- deleting the final row of an atom removes the atom while preserving other projected members.

**Result.** The common observable-support partition is now a production substrate while old physical families remain available for differential validation.

## Integration boundary

Pass78 intentionally implements only Γ-SAMF Stage A/B.

Integrated:

1. generic finite support-atom fabric in `kernel-semantics`;
2. production `MaterializedObservableAtomState` over real `PhysicalRowId`;
3. current-API probe/count adapters and legacy oracle parity;
4. relation-delta maintenance and COW publication;
5. retained-memory accounting for the new reconstructible state.

Not integrated yet:

1. replacement of `SemanticIndex`, `SemanticStatistics`, `SemanticQuotientFactor`, maintained Group/TopK or I64 paths;
2. unified `ObservableDemand` advisor / global capability frontier;
3. exact/pruned Pareto selection under hard budgets;
4. durable SAMF recipes/recovery;
5. generated differential maintenance IR;
6. ordered/annotation/specialized-I64 overlays as the sole physical representation.

This staging is deliberate: deleting the legacy families before randomized/crossover equivalence would remove the oracle needed to falsify the migration.

## Γ-GCC R&D orientation

`CFMD_RND_GAMMA_GROUNDED_CLOSURE_CALCULUS_2026-09-21(1).zip` was reviewed after the Pass78 freeze and **not merged**.

Its core claim is compatible with Pass77/78: APNF determinant saturation, lifecycle reachability and finite positive recursive support are all instances of grounded least closure over finite hyperrules. The prototype reports parity against Pass77 fixed-point/lifecycle implementations and hostile n-ary/cycle cases. The recommended next integration order is sensible: shared explicit grounded-closure substrate first, determinant closure first consumer, then fixpoint/lifecycle parity, and only later recursive query support. Unrestricted recursive Bag/Natural multiplicity must not be admitted without a certified finite-height/idempotent carrier.

See `PASS78_GAMMA_GCC_RND_ORIENTATION.md`.

## CLOSED exactly in Pass78

1. recovery/admission key-work accounting was size-blind for large Text/structural cells;
2. recovery scheduling lacked a physical-layout-independent semantic structural work metric;
3. there was no common production support-atom substrate on which exact semantic fibers/statistics can be represented without duplicating physical row placement across every future observable capability;
4. support-atom identity initially risked creating a redundant local namespace; production instead reuses the product observable's catalog-local `EqClassId` and rejects cross-catalog aliasing.

These are concrete substrate/resource closures. They do **not** close the complete historical unified-lifecycle/advisor or general multiway items.

## Advanced but NOT CLOSED

1. 🟨 **Unified observable physical lifecycle.** SAMF now exists beside legacy states, but advisor ownership, durable recipes, recovery and family retirement are not unified yet.
2. 🟨 **Recovery economics.** Admission combines key-cell count, structural semantic work and retained bytes, with deferred continuation, but still lacks workload-benefit/background scheduling.
3. 🟨 **Exact memory pressure.** Retained-size estimators include SAMF but remain planning estimates, not allocator/RSS/shared-backing truth.
4. 🟨 **Differential compiler.** Relation delta maintenance exists for SAMF, but a generic measure/APNF maintenance IR has not replaced family-specific update code.
5. 🟨 **General multiway mathematics.** Pass77 APNF/SAMF substrate advances the representation, but complete residual pullback/executor mathematics remains in the R&D branch.
6. 🟨 **Grounded closure unification.** Γ-GCC is reviewed only; determinant/fixpoint/lifecycle implementations remain separate production engines.

## Historical OPEN accounting

Pass77 ended with **22 active historical OPEN**.

- Historical OPEN fully closed in Pass78: **0 / 22**;
- genuinely new historical OPEN: **0**;
- total active OPEN after Pass78: **22**.

The active historical frontier remains:

1. structural/custom semantic physical persistence and structural ordering;
2. general nested/multiway/bushy execution (`RelExpr → APNF`, residual pullback, lifecycle/performance closure);
3. unified cross-family/observable physical lifecycle and advisor;
4. autonomous workload telemetry, correlations, decay/hysteresis and scheduling;
5. exact resident/shared memory pressure accounting beyond deterministic retained-byte estimates;
6. remaining physical layouts plus OrderedView/pagination;
7. recovery rebuild economics beyond bounded deterministic admission/continuation;
8. durable revision DAG / branch+merge ancestry;
9. general historical durable-format migration;
10. arbitrary/plugin semantic executable packaging/signing/deployment;
11. transaction intent/outcome retention and GC;
12. streaming/chunked checkpoints and metadata;
13. real power-loss plus Windows/network-FS/FUSE assurance;
14. general lock-poison/restart policy;
15. group commit / async durability;
16. replication / consensus / broader distribution;
17. durable-store authentication/MAC;
18. formal rename/fsync/GC power-loss proof;
19. transaction repair runtime;
20. remaining formal mechanization;
21. maintained I64 Group constant-factor debt;
22. maintained I64 TopK constant-factor debt.

## Verification

Rust: `rustc 1.98.1 (48a229cea 2026-09-01)`.

Frozen-source gate PASS:

- `cargo fmt --all -- --check`;
- `cargo check --workspace --all-targets`;
- `cargo test --workspace --all-targets` — **483 passed / 0 failed / 8 ignored**;
- `cargo clippy --workspace --all-targets -- -D warnings`;
- `cargo test --workspace --all-targets --release` — **483 / 0 / 8**;
- `cargo build --workspace --release`;
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps`;
- `RUSTFLAGS='-C overflow-checks=yes' cargo test --workspace --all-targets --release` — **483 / 0 / 8**.

To avoid losing completed work to the external command-window timeout, cold release/overflow fingerprints were first warmed and verified package-by-package with a shared target. The exact workspace commands were then rerun on the warmed target and completed successfully. Package logs remain under `evidence/pass78/release_batches/` and `evidence/pass78/overflow_batches/`.

`evidence/pass78/SOURCE_FREEZE.sha256` and `SOURCE_POSTGATE.sha256` are byte-identical.

Frozen snapshot:

- **491 declared tests**;
- **204 `kernel-plan` declared tests**;
- **54 `kernel-semantics` declared tests**;
- **21 crates**;
- **69,364 Rust LOC**;
- **0 external registry/git Cargo sources**;
- **0 unsafe hits**;
- **19 existing `#[allow(...)]`**, no new suppression;
- **0 TODO/FIXME/todo!/unimplemented! hits**.

## Next step

Do not delete legacy physical families in the next mechanical step. The clean next mainline migration is SAMF Stage C: introduce capability overlays/current-API adapters under randomized oracle comparison, then a global `ObservableDemand`/Pareto advisor and durable recipe boundary. In parallel, Γ-GCC can be integrated as a shared semantic closure substrate without entangling the SAMF physical migration.
