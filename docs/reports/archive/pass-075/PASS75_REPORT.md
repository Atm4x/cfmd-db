# PASS75 REPORT — bounded advisor-owned physical reconstruction

Status: **VERIFIED**.

Source freeze: **2026-09-21 12:55:45 UTC**.

Production diff relative to Pass74:

- `crates/kernel-plan/src/lib.rs`

Pass75 intentionally does **not** advance the general nested/multiway JOIN executor. That mathematical/execution frontier is being attacked independently by the R&D program; this pass targets the orthogonal physical recovery/lifecycle boundary.

## Problem → hypothesis → implementation → falsification → result

### 1. Durable derived artifacts could create an unbounded synchronous reopen burst

**Problem.** Pass71/74 made physical recipes reconstructible, but `DurableRuntime::open()` rebuilt every compatible optional recipe immediately. Recovery therefore had no explicit admission boundary for advisor-owned I64 indexes, semantic indexes, Γ-QCN factors or semantic statistics. A logically small restart could pay an arbitrarily large derived-state rebuild before the runtime became available.

**Hypothesis.** Recovery should preserve the same ownership distinction as the live lifecycle advisor: explicit/manual physical intent is fixed, while advisor-owned derivatives may be omitted and reconstructed later without changing authoritative `Revision=(S,Γ,M)`. A deterministic recovery policy can therefore bound advisor reconstruction while remaining semantics-neutral.

**Implementation.** `PhysicalRecoveryPolicy` adds three admission ceilings: advisor rebuild key evaluations, total estimated retained bytes and advisor estimated retained bytes. Relation layouts and manual pins rebuild first as fixed intent. Advisor-owned recipes are considered deterministically afterward. `PhysicalStore::restore_durable_physical_artifacts` reconstructs a candidate store, checks the configured ceilings and accepts or discards only the derived candidate. Existing `DurableRuntime::open()` uses an unlimited default policy for compatibility; `open_with_recovery_policy()` exposes the bounded path.

The work estimate is intentionally `rows × max(key_parts, 1)` for applicable artifacts. It is a deterministic pre-build key-evaluation proxy, **not** a claim of exact CPU work. Likewise retained-byte accounting is the existing deterministic artifact estimate, not allocator/RSS truth.

**Falsification.** A zero advisor-work budget with both a manual semantic index and advisor-owned I64 recipe must preserve the manual pin and skip only the advisor recipe. A zero total estimated-byte ceiling must be allowed to reject an optional advisor artifact while recovered logical Revision remains exactly equal. Existing stale-recipe hostiles still require fail-open physical behavior and exact logical recovery.

**Result.** Recovery now has an explicit semantics-neutral resource boundary for advisor-owned derived state instead of unconditional eager reconstruction.

### 2. Bounded recovery initially threatened durable manual-pin intent

**Problem.** The first candidate policy applied a global work ceiling uniformly. That could skip a manually pinned durable physical recipe. If the reopened runtime subsequently checkpointed, the skipped pin could disappear from future durable recipes, turning a transient resource policy into mutation of explicit physical intent.

**Hypothesis.** Manual vs advisor ownership must survive recovery exactly as it survives online lifecycle decisions: budgets may evict/admit advisor state, but cannot silently revoke a manual pin.

**Implementation.** Manual recipes are sorted/rebuilt as fixed physical intent before advisor admission. Advisor key-evaluation budgets do not charge or reject them. Their estimated bytes do count against the total ceiling available to later advisor artifacts, so fixed intent can naturally leave no budget for optional reconstruction without itself being dropped.

**Falsification.** `bounded_physical_recovery_prioritizes_manual_pin_over_advisor_recipe` runs under zero advisor rebuild budget and proves the manual index exists after reopen while the advisor recipe appears in the explicit skipped set.

**Result.** Recovery resource control cannot erase manual durable physical intent.

### 3. Recovery outcomes and supervisor policy were not lineage-visible

**Problem.** Before Pass75, stale recipe drops and future budget skips had no structured reconstruction report. More importantly, a policy applied only to an initial `DurableRuntime::open` would not be sufficient: `DurableRuntimeSupervisor` performs later explicit/automatic reopen after fail-stop, and could silently return to the legacy unlimited policy.

**Hypothesis.** Recovery policy belongs to the runtime supervisor lineage, and reconstruction disposition must be observable without promoting derived state to authority.

**Implementation.** `PhysicalRecoveryReport` records rebuilt recipes, key-evaluation-budget skips, estimated-byte-budget skips, incompatible/stale drops and aggregate work/byte accounting. `DurableRuntimeSupervisor` stores `PhysicalRecoveryPolicy`; `create_with_recovery_policy`, `open_with_recovery_policy` and internal reopen paths all preserve it. `recover_with_report()` exposes each reopen result while the legacy `recover()` keeps the prior convenience surface.

**Falsification.** A supervisor with zero advisor-work budget is recovered repeatedly and must skip the same advisor semantic index on every reopen. A stale durable recipe must be reported in `dropped_incompatible` while logical recovery succeeds unchanged.

**Result.** The selected recovery resource policy is stable across process-level recovery lineage and reconstruction decisions are inspectable rather than silent.

## CLOSED exactly in Pass75 — narrower production defects

1. advisor-owned durable physical derivatives were reconstructed eagerly on reopen without an explicit recovery admission/resource policy;
2. bounded recovery had no defined ownership law protecting manual durable pins from policy-driven disappearance;
3. physical reconstruction disposition was opaque and supervisor reopen could not retain a selected recovery policy.

These are concrete production closures, not closure of the broader historical recovery-economics item.

## Historical OPEN accounting

Pass74 ended with **22 active historical OPEN**.

- Historical OPEN fully closed in Pass75: **0 / 22**;
- genuinely new OPEN: **0**;
- total active OPEN after Pass75: **22**.

The recovery-economics item remains OPEN for benefit-ranked/background scheduling, faster reconstruction paths, structural-key-size-aware costing, allocator/RSS/external-pressure accounting and arbitrary future physical families. The cross-family lifecycle/advisor item also remains broader than reopen admission.

The general nested/multiway/high-width JOIN item remains deliberately untouched in Pass75 because the independent R&D program is attacking JOIN mechanics at the mathematical-kernel level rather than incrementally extending the current GYO/QCN route.

## Verification

Rust: `rustc 1.98.1 (48a229cea 2026-09-01)`.

Frozen-source final gate PASS:

- `cargo fmt --all -- --check`;
- `cargo check --workspace --all-targets`;
- `cargo test --workspace --all-targets` — **453 passed / 0 failed / 8 ignored**;
- `cargo clippy --workspace --all-targets -- -D warnings`;
- `cargo test --workspace --all-targets --release` — **453 / 0 / 8**;
- `cargo build --workspace --release`;
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps`;
- `RUSTFLAGS='-C overflow-checks=yes' cargo test --workspace --all-targets --release` — **453 / 0 / 8**.

The first cold release and overflow-release attempts exceeded the external command window while compiling and were not counted as PASS or FAIL. Their warmed full workspace invocations completed successfully with exit code 0.

Freeze/post-gate `crates/` SHA-256 inventories are byte-identical.

Frozen snapshot:

- **461 declared tests**;
- **192 `kernel-plan` declared tests**;
- **21 crates**;
- **65,087 Rust LOC**;
- **0 external registry/git Cargo sources**;
- **0 unsafe hits**;
- **19 existing `#[allow(...)]`**, no new suppression;
- **0 TODO/FIXME/todo!/unimplemented! hits**.

## Продвинуто, но НЕ CLOSED

1. 🟨 **Recovery rebuild economics.** Reopen now has deterministic advisor admission and observability, but no benefit-ranked/background scheduler, raw/fast reconstruction format or structural-key-size-aware cost model.
2. 🟨 **Cross-family physical lifecycle/advisor.** Online and recovery ownership/resource boundaries are closer to one law, but counterfactual benefit, write-maintenance cost, arbitrary physical families and autonomous scheduling are still fragmented.
3. 🟨 **Exact physical memory accounting.** Admission uses deterministic retained-byte estimates; allocator overhead, shared-backing truth, RSS and external pressure are not exact.

## Осталось OPEN

Authoritative historical ledger after Pass75 — **22 active OPEN**:

1. ⬜ Structural/custom-equivalence physical indexing: durable structural-index persistence/rebuild, arbitrary/plugin canonical laws and structural ordering.
2. ⬜ General nested/multiway/bushy/high-width JOIN planning/execution beyond the currently verified bounded Γ-QCN families.
3. ⬜ First-class cross-family physical advisor/lifecycle spanning specialized I64, generic semantic indexes, statistics, Γ-QCN factors/support, algebraic layouts and future families.
4. ⬜ Autonomous workload telemetry/statistics: histograms, correlation, decay/hysteresis and lifecycle scheduling.
5. ⬜ Exact byte/resident-memory accounting, shared-backing budgeting, external-pressure integration and rebuild scheduling.
6. ⬜ Remaining physical layouts beyond current row/column/algebraic families, plus explicit `OrderedView`/pagination.
7. ⬜ Recovery rebuild economics beyond Pass75 admission: benefit-ranked/background scheduling, faster reconstruction and structural-key-size-aware costing.
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
21. ⬜ Maintained I64 Group constant-factor gap (performance debt, not correctness debt).
22. ⬜ Maintained I64 TopK constant-factor gap (performance debt, not correctness debt).

Historical #6 from Pass62 (canonical-key/cache encoding/version migration) was closed in Pass69; historical #9 from Pass62 (persistent outer physical artifact catalogs) was closed in Pass67. Pass75 does not renumber these closures back into the active ledger.

## Следующий шаг

Do not duplicate the independent Γ-QSFJ/JOIN R&D until its closeout arrives. The strongest orthogonal Pass76 target is to extend recovery admission into a **benefit-ranked reconstruction scheduler** that can defer advisor-owned artifacts instead of merely accepting/skipping them synchronously, while preserving the same manual/advisor authority law. A second viable branch is exact/shared physical memory accounting if allocator/RSS truth can be introduced without contaminating semantic authority.
