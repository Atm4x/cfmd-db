# PASS76 REPORT — continuable, work-aware physical recovery

Status: **VERIFIED**.

Source freeze: **2026-09-21 13:23:46 UTC**.

Production diff relative to Pass75:

- `crates/kernel-plan/src/lib.rs`

Pass76 intentionally does **not** integrate or extend the independent native-multiway/JOIN R&D branch. `PASS76_NATIVE_MULTIWAY_RND_ORIENTATION.md` records only forward-compatibility constraints extracted from that R&D input.

## Problem → hypothesis → implementation → falsification → result

### 1. Bounded recovery admission still depended on arbitrary recipe identity order

**Problem.** Pass75 separated manual and advisor ownership, but advisor recipes with the same ownership class were still considered in ordinary durable-spec ordering. Under a limited key-evaluation budget, a larger rebuild with a lexicographically earlier relation/recipe could consume the budget before a much cheaper rebuild. Resource allocation therefore depended on `SemanticId`/recipe ordering rather than the resource quantity the policy claims to control.

**Hypothesis.** Manual/fixed intent must remain first. Within advisor-owned state, the deterministic additive key-evaluation budget should admit cheaper compatible rebuilds before more expensive ones. This maximizes the number of admitted recipes under that one budget without inventing a workload-benefit score.

**Implementation.** Recovery now preflights optional recipes for compatibility and deterministic rebuild key-evaluation work. Incompatible recipes are reported and removed before scheduling. Compatible manual recipes retain stable recipe order and unconditional priority; compatible advisor recipes are sorted by increasing key-evaluation work and then stable recipe identity.

**Falsification.** A two-relation hostile deliberately assigns the expensive ten-row advisor index the smaller semantic relation id and the cheap two-row index the larger id. With advisor work budget `2`, only the cheap index rebuilds and the expensive one is reported as skipped.

**Result.** Recovery key-work budgeting is no longer accidentally controlled by semantic identifier ordering.

### 2. Pass75 budget skips were not continuable on the serving runtime

**Problem.** Pass75 could make reopen fast by skipping compatible advisor-owned derivatives, but its production surface offered no way to resume those exact deferred recipes after the runtime began serving. The only generic reconstruction path was another reopen or unrelated future workload advice.

**Hypothesis.** Because the skipped state is reconstructible and non-authoritative, it can be retried against the current immutable runtime root under a new recovery policy. Successful reconstruction should publish one new physical root version while retaining the exact same authoritative Revision.

**Implementation.** `PhysicalRecoveryReport::deferred_advisor_artifacts()` derives a deterministic deduplicated continuation set from budget skips only. `DurableRuntime::resume_deferred_physical_recovery` and the supervisor equivalent retry that set against the current pinned Γ/relation layouts. Manual recipes are filtered out even if a caller fabricates a report. If nothing compatible rebuilds, no root is republished. If at least one derivative rebuilds, publication is a normal immutable root swap under the same logical Revision.

**Falsification.** A budget-zero reopen defers an advisor semantic index. Unlimited continuation rebuilds it, preserves the Revision id and advances the runtime root version by exactly one. Checkpoint/reopen proves the rebuilt state returns to the ordinary durable recipe lifecycle. A hostile fabricated report containing a manual recipe plus an incompatible advisor recipe cannot replay the manual artifact, reports the incompatible recipe, and does not advance root version. Supervisor continuation works without another reopen.

**Result.** Bounded recovery can now separate startup latency from optional reconstruction work instead of making each compatible skip terminal for the current serving runtime.

## CLOSED exactly in Pass76

1. recovery key-evaluation budget allocation depended on durable recipe/SemanticId ordering rather than rebuild work;
2. compatible advisor recipes skipped during bounded reopen had no production continuation path on the serving runtime.

These are concrete recovery-economics closures, not closure of the broader historical recovery/advisor item.

## Historical OPEN accounting

Pass75 ended with **22 active historical OPEN**.

- Historical OPEN fully closed in Pass76: **0 / 22**;
- genuinely new OPEN: **0**;
- total active OPEN after Pass76: **22**.

The recovery-economics item remains OPEN for benefit-ranked/autonomous scheduling, structural-key-size-aware costing, faster/raw reconstruction, exact pressure accounting and arbitrary future physical families. The general multiway/JOIN item remains deliberately external to this pass while native Γ-lattice/determinant R&D is independently falsified.

## Verification

Rust: `rustc 1.98.1 (48a229cea 2026-09-01)`.

Frozen-source final gate PASS:

- `cargo fmt --all -- --check`;
- `cargo check --workspace --all-targets`;
- `cargo test --workspace --all-targets` — **457 passed / 0 failed / 8 ignored**;
- `cargo clippy --workspace --all-targets -- -D warnings`;
- `cargo test --workspace --all-targets --release` — **457 / 0 / 8**;
- `cargo build --workspace --release`;
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps`;
- `RUSTFLAGS='-C overflow-checks=yes' cargo test --workspace --all-targets --release` — **457 / 0 / 8**.

Cold release and overflow-release compilation attempts that exceeded command windows were not counted as PASS or FAIL. After warming the corresponding fingerprints, the full commands completed successfully.

Freeze/post-gate `crates/` SHA-256 inventories are byte-identical.

Frozen snapshot:

- **465 declared tests**;
- **196 `kernel-plan` declared tests**;
- **21 crates**;
- **65,481 Rust LOC**;
- **0 external registry/git Cargo sources**;
- **0 unsafe hits**;
- **19 existing `#[allow(...)]`**, no new suppression;
- **0 TODO/FIXME/todo!/unimplemented! hits**.

## Продвинуто, но НЕ CLOSED

1. 🟨 **Recovery rebuild economics.** Startup and continuation are now resource-aware and work-ordered, but there is no persisted workload-benefit score, autonomous/background executor or write-maintenance pricing.
2. 🟨 **Cross-family physical lifecycle/advisor.** Recovery scheduling obeys the same manual/advisor authority distinction, but online and restart benefit/cost models are still fragmented across artifact families.
3. 🟨 **Recovery work accounting.** Key-evaluation count is deterministic but not structural-key-size-aware CPU work; retained-byte estimates are still planning estimates, not allocator/RSS truth.
4. 🟨 **Native multiway mathematical kernel.** Γ-LCF/virtual-law R&D was reviewed as a future constraint only. Production JOIN IR/execution is unchanged in Pass76.

## Осталось OPEN

Authoritative historical ledger after Pass76 — **22 active OPEN**:

1. ⬜ Structural/custom-equivalence physical indexing: durable structural-index persistence/rebuild, arbitrary/plugin canonical laws and structural ordering.
2. ⬜ General nested/multiway/bushy/high-width JOIN planning/execution beyond the currently verified bounded Γ-QCN families.
3. ⬜ First-class cross-family physical advisor/lifecycle spanning specialized I64, generic semantic indexes, statistics, Γ-QCN factors/support, algebraic layouts and future families.
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

Do not duplicate the independent native-multiway/JOIN R&D until its non-enumerative hostile closeout arrives. The strongest orthogonal Pass77 direction is to make recovery cost estimation structural-key-size-aware and/or introduce a unified reconstructible scheduling contract that can consume real workload benefit without persisting advisory state as semantic authority. Exact/shared physical-memory pressure is the other clean branch.
