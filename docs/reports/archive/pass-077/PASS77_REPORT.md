# PASS77 REPORT — Semantic Observable Foundation + Γ Anchor-Pullback substrate

Status: **VERIFIED**.

Source freeze: **2026-09-21 14:30:44 UTC**.

Production diff relative to Pass76:

- `crates/kernel-types/src/lib.rs`
- `crates/kernel-semantics/src/lib.rs`
- `crates/kernel-semantics/src/observable.rs` (new)
- `crates/kernel-semantics/src/anchor_pullback.rs` (new)
- `crates/kernel-semantic-index/src/lib.rs`
- `crates/kernel-plan/src/algebraic_native.rs`
- `crates/kernel-plan/src/lib.rs`

R&D input was reviewed and manually integrated; no standalone prototype source was mechanically merged.

## Problem → hypothesis → implementation → falsification → result

### 1. Coarse unordered structural Γ keys were not closed under their own durable canonical-shape law

**Problem.** `Set` / `Bag` / `Map` canonicalization sorted child canonical keys but did not aggregate collisions created by a coarser child equivalence. Two physically different representatives such as `"A"` and `"a"` could therefore produce duplicate canonical atoms. Production could construct such a key while the durable canonical decoder correctly rejected that shape as non-canonical.

**Hypothesis.** Unordered structural semantics should have one finite-measure normal form: canonical atom plus explicit occurrence multiplicity. Bag's stored count remains part of its atom; measure multiplicity records how many physical entries collapse to the same semantic atom.

**Implementation.** `CanonicalEqKey::{Set,Bag,Map}` now carry ordered finite counting measures. Structural and native/algebraic canonicalization use the same aggregation law. The durable key grammar is explicitly versioned as v2; semantic-index compatibility revision is bumped in lockstep.

**Falsification.** Hostile coarse case-insensitive Set/Bag/Map fixtures create physical representative collisions, round-trip through the v2 codec and verify multiplicity. Non-canonical ordering/zero/duplicate shapes remain rejected, previous/future codec revisions fail closed, and a fixed v2 golden byte sequence protects the representation from Rust-layout drift.

**Result.** Production no longer emits a durable canonical key that its own decoder rejects; unordered structural equality has one constructor-independent finite-measure representation law.

### 2. Γ quotient coordinates had no explicit revision-local nominal identity layer

**Problem.** `CanonicalEqKey` is the exact semantic value but is too heavy to use as every query-local coordinate identity. Reusing ordinary integer IDs without ownership would allow classes/observables from different realizations to alias accidentally.

**Hypothesis.** Separate semantic definition from revision-local realization. Observable/class IDs may be compact nominal coordinates only if they are bound to one pinned semantic revision and one catalog instance and remain reconstructible.

**Implementation.** Added `RevisionObservableCatalog`, `RevisionObservableId`, `EqClassId`, pinned-equivalence observables and generic product observables. Catalog instance identity is embedded into nominal IDs. Product class construction validates component ownership. `CertifiedSemanticMorphism` is n-ary -> m-ary from the start; composition and product projections preserve the same revision/catalog boundary.

**Falsification.** Hostiles cover revision mismatch, product meet semantics, n→m functionality, conflicting images, morphism composition and two independent catalogs of the same semantic revision. Cross-catalog observable/class aliases are impossible.

**Result.** The future multiway kernel has a safe query-local semantic coordinate layer without promoting physical quotient IDs to durable authority.

### 3. Hyper-determinants and deterministic closure had no production semantic normal form

**Problem.** Unary morphism graphs/SCCs cannot represent true hypercycles such as `AB→C`, `AC→B`, `BC→A`. Requiring a minimum FD cover/candidate key would also move an NP-hard optimization obligation into the semantic compiler.

**Hypothesis.** Product observables collapse hyper-determinants into ordinary finite partial morphisms. The unique semantic object is the least closure `Cl_D`; execution can compile that closure to an incidence worklist. A reverse-delete generator only needs to be deterministic and inclusion-minimal.

**Implementation.** Added `DeterminantTheory` over certified morphisms, incidence/worklist closure, deterministic stable inclusion-minimal generator construction and value-level morphism saturation with exact undefined-source/conflict rejection. Direct/transitively redundant morphisms are retained as legal accelerators rather than deleted from physical state.

**Falsification.** A true three-coordinate hypercycle requires a two-coordinate generator; every strict subset fails. A 128-coordinate reversed chain closes with 126 incidence updates and 127 target attempts instead of repeated full-rule rescans. Value propagation succeeds independent of dependency ordering and rejects conflicting exact assignments.

**Result.** Hyper-determinants no longer require unary special cases or a canonical/minimum FD-list authority.

### 4. Finite relation factors lacked one lossless semantic representation for deterministic and residual work

**Problem.** General relation support was still conceptually treated as rows plus a later executor choice. The Γ-APNF R&D shows that every finite factor can first be represented as an anchor measure plus deterministic reconstruction; only compatibility between anchors remains free.

**Hypothesis.** For each finite factor, reverse-delete coordinates while projection remains injective on distinct support. The resulting inclusion-minimal basis indexes a weighted anchor measure; reconstruction to all removed coordinates is a certified morphism.

**Implementation.** Added `RevisionFiniteMeasure`, weighted duplicate aggregation, safe finite-factor hyper-determinant derivation, `AnchorMeasureState`, lossless reconstruction morphisms and the query-local `AnchorPullbackNormalForm`. A determinant branch-free certificate is emitted only when one concrete anchor basis closes the entire APNF coordinate set.

**Falsification.** Weighted factors reconstruct byte-for-value/class exactly, duplicate support rows merge multiplicity, non-functional projections refuse certification, arbitrary source/target ordering is preserved, APNF rejects another catalog realization, and a singleton factor correctly reduces to an empty anchor with a constant reconstruction.

**Result.** The mathematical substrate needed by a future Γ-APNF general executor is now production code without switching current JOIN execution.

## CLOSED exactly in Pass77

1. production unordered structural canonical keys could violate their own durable canonical-shape invariant under coarse Γ collisions;
2. no explicit nominal revision-local observable/class coordinate boundary existed for the new semantic quotient layer;
3. production morphism substrate was not n-ary → m-ary / hyper-determinant ready;
4. no order-independent least determinant-closure/worklist substrate existed;
5. finite semantic factors had no production lossless weighted anchor-measure + reconstruction representation;
6. query-local APNF foundation and proof-carrying determinant branch-free certificate were absent.

These closures are substrate/correctness closures. They do **not** close the historical general multiway execution item.

## Historical OPEN accounting

Pass76 ended with **22 active historical OPEN**.

- Historical OPEN fully closed in Pass77: **0 / 22**;
- genuinely new OPEN: **0**;
- total active OPEN after Pass77: **22**.

The general multiway item remains OPEN for production `RelExpr → APNF` lowering, residual anchor-pullback execution, incremental/durable APNF derivatives and crossover/performance evidence. Structural/custom physical persistence and structural ordering also remain OPEN despite the new exact finite-measure semantic law.

## Verification

Rust: `rustc 1.98.1 (48a229cea 2026-09-01)`.

Frozen-source final gate PASS:

- `cargo fmt --all -- --check`;
- `cargo check --workspace --all-targets`;
- `cargo test --workspace --all-targets` — **471 passed / 0 failed / 8 ignored**;
- `cargo clippy --workspace --all-targets -- -D warnings`;
- `cargo test --workspace --all-targets --release` — **471 / 0 / 8**;
- `cargo build --workspace --release`;
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps`;
- `RUSTFLAGS='-C overflow-checks=yes' cargo test --workspace --all-targets --release` — **471 / 0 / 8**.

Cold release compilation exceeded one command window. Overflow-release required two warm-up attempts before the complete command finished. These timeout attempts are neither PASS nor FAIL; only the completed invocations above are recorded as gate results.

Freeze/post-gate `crates/` SHA-256 inventories are byte-identical.

Frozen snapshot:

- **479 declared tests**;
- **196 `kernel-plan` declared tests**;
- **50 `kernel-semantics` declared tests**;
- **21 crates**;
- **67,589 Rust LOC**;
- **0 external registry/git Cargo sources**;
- **0 unsafe hits**;
- **19 existing `#[allow(...)]`**, no new suppression;
- **0 TODO/FIXME/todo!/unimplemented! hits**.

## Продвинуто, но НЕ CLOSED

1. 🟨 **General multiway/JOIN.** The mathematical semantic substrate is now materially stronger: exact observables, finite measures, hyper-determinant closure and anchor factorization exist in production. `RelExpr → APNF` and the residual pullback engine are still absent.
2. 🟨 **Structural/custom semantic persistence.** Constructor-independent finite-measure equality is fixed and versioned, but durable structural index payloads, arbitrary/plugin semantic law deployment and structural ordering remain OPEN.
3. 🟨 **APNF lifecycle.** Anchor measures, determinant closure indexes and composed morphisms are reconstructible only; incremental maintenance, durable recipes, advisor ownership and memory budgeting are not integrated yet.
4. 🟨 **Residual freedom.** The R&D exact fiber-profile theorem is accepted mathematically, but production has only the determinant-backed branch-free certificate until a residual engine can produce/check the full pullback witness object.

## Осталось OPEN

Authoritative historical ledger after Pass77 — **22 active OPEN**:

1. ⬜ Structural/custom-equivalence physical indexing: durable structural-index persistence/rebuild, arbitrary/plugin canonical laws and structural ordering.
2. ⬜ General nested/multiway/bushy/high-width JOIN planning/execution; now expected to migrate toward Γ-APNF rather than grow more semantic special cases around GYO/QCN.
3. ⬜ First-class cross-family physical advisor/lifecycle spanning specialized I64, generic semantic indexes, statistics, Γ-QCN/APNF derivatives, algebraic layouts and future families.
4. ⬜ Autonomous workload telemetry/statistics: histograms, correlation, decay/hysteresis and lifecycle scheduling.
5. ⬜ Exact byte/resident-memory accounting, shared-backing budgeting, external-pressure integration and rebuild scheduling.
6. ⬜ Remaining physical layouts beyond current row/column/algebraic families, plus explicit `OrderedView`/pagination.
7. ⬜ Recovery rebuild economics beyond Pass76: benefit-ranked/autonomous scheduling, faster reconstruction and structural-key-size-aware costing.
8. ⬜ Durable revision DAG / branch+merge ancestry and merge replay.
9. ⬜ General historical durable-format migration framework.
10. ⬜ Arbitrary/plugin semantic executable artifact packaging/signing/authentication/deployment.
11. ⬜ Transaction intent/outcome retention + GC.
12. ⬜ Streaming/chunked checkpoints and metadata.
13. ⬜ Real machine power-loss assurance plus Windows/network-FS/FUSE durability semantics.
14. ⬜ General lock-poison/restart policy.
15. ⬜ Group commit / async durability.
16. ⬜ Replication / consensus and broader distribution architecture.
17. ⬜ Durable-store authentication/MAC; CRC32C remains corruption detection only.
18. ⬜ Formal power-loss proof for rename/fsync/GC protocol.
19. ⬜ Transaction repair runtime.
20. ⬜ Formal mechanization of remaining semantic/transport/retention/power-loss obligations.
21. ⬜ Maintained I64 Group constant-factor gap.
22. ⬜ Maintained I64 TopK constant-factor gap.

## Следующий шаг

The mathematically natural next multiway work is now engineering rather than another representation patch: compile real query factors into `RevisionFiniteMeasure`/APNF, add incremental anchor/morphism maintenance and implement one residual anchor-pullback executor with exact oracle comparison and crossover benchmarks against the retained QCN/GYO paths.

If that work stays with the independent R&D agent, the main implementation branch can continue orthogonally on APNF derivative lifecycle/resource accounting or the residual Group/TopK performance debt.
