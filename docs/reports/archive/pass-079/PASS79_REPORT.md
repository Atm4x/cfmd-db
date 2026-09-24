# PASS79 REPORT — Γ Grounded Closure Calculus substrate

Status: **VERIFIED**.

Source freeze: **2026-09-21 16:57:25 UTC**.

Production diff relative to Pass78:

- `Cargo.toml`
- `Cargo.lock`
- `crates/kernel-grounded-closure/Cargo.toml` (new)
- `crates/kernel-grounded-closure/src/lib.rs` (new)
- `crates/kernel-semantics/Cargo.toml`
- `crates/kernel-semantics/src/anchor_pullback.rs`
- `crates/kernel-fixpoint/Cargo.toml`
- `crates/kernel-fixpoint/src/lib.rs`
- `crates/kernel-lifecycle/Cargo.toml`
- `crates/kernel-lifecycle/src/lib.rs`

## Problem → hypothesis → implementation → falsification → result

### 1. Least grounded closure existed as several separate semantic engines

**Problem.** Pass78 had an APNF determinant worklist, a separate `kernel-fixpoint` BFS semantics, and a separate lifecycle liveness implementation. Their shared least-grounded-closure law was only an external R&D observation.

**Hypothesis.** Finite idempotent support admits one semantic primitive: seeds plus finite hyperrules, with least grounded closure certified by strict-rank witnesses. Physical lowerings may remain specialized.

**Implementation.** Added dependency-minimal `kernel-grounded-closure`. `GroundedProgram` contains a finite dense atom universe, normalized hyperrules and seeds. `solve` uses an incidence worklist. `GroundedCertificate` stores liveness, rank and one selected witness. `check` independently verifies seed inclusion, rule closure and strict-rank grounded proofs.

**Falsification.** 2,000 random hyperprograms match a naïve fixed-point oracle. A groundless two-cycle stays dead. A 2-premise rule does not fire from one premise. Certificate checking rejects ungrounded support by construction.

**Result.** CFMD has one production finite grounded-closure semantic substrate instead of duplicating this law per subsystem.

### 2. Deletion of recursive support cannot be treated as ordinary support-count decrement

**Problem.** A cycle can mutually support counts after its only grounded proof disappears. Plain zero-crossing support counts are insufficient for recursive closure deletion.

**Hypothesis.** Invalidate only the cone reachable through the currently selected grounded witnesses, preserve everything outside that cone, then locally re-fixpoint the affected region.

**Implementation.** Γ-GCC includes selected-witness reverse dependencies, `GroundedUpdate`, insertion propagation and witness-cone local deletion re-evaluation.

**Falsification.** Across 5,000 random mixed seed/rule updates the incremental result equals a full rebuild. Deleting a non-selected redundant rule yields zero affected atoms. Removing the only grounding seed from a self-supported cycle removes the whole cycle.

**Result.** A reusable exact dynamic deletion law now exists for future lifecycle/recursive-support consumers.

### 3. APNF determinant closure had its own worklist implementation

**Problem.** `DeterminantTheory::closure` encoded the same hyperrule calculus locally, duplicating solver semantics and future certificate/update work.

**Hypothesis.** Certified semantic morphisms can remain the authority while their source/target observable sets compile to dense grounded hyperrules.

**Implementation.** `DeterminantTheory` now owns a revision-local observable↔dense-atom lowering, compiled Γ-GCC rules and a reusable incidence index. Closure execution delegates to Γ-GCC. Multi-target morphisms lower losslessly to one rule per target.

**Falsification.** Existing true-hypercycle/APNF tests pass unchanged. The historical work-accounting test still reports exactly `126 incidence_updates / 127 target_attempts`; the adapter preserves the old convention that initial seed incidences are not charged.

**Result.** Determinant semantics are unified without changing observable authority or its public diagnostics.

### 4. Reachability used a separate BFS semantic solver

**Problem.** `kernel-fixpoint` expressed the unary special case of Γ-GCC but had an independent solver path.

**Hypothesis.** Reachability can compile to one-premise rules while keeping the existing certificate type and checker.

**Implementation.** `kernel-fixpoint::solve` now lowers entities to dense grounded atoms and edges to unary rules, executes Γ-GCC, then reconstructs `reachable`, `rank` and `parent` from grounded witnesses.

**Falsification.** The existing adjacency-lowering hostile compares the entire certificate, not only reachability, against the previous relation-scan BFS oracle over 32 generated graphs. It passes unchanged. The existing independent `check` remains the verifier used by `CheckedCertificate`.

**Result.** APNF determinant saturation and general reachability now share one semantic fixed-point kernel.

### 5. Lifecycle should share semantics, not necessarily implementation

**Problem.** Replacing `MaintainedDenseLifecycle` mechanically with a generic hyperrule engine would throw away a useful specialized dense physical lowering.

**Hypothesis.** Keep the specialized implementation, but pin its semantic law to Γ-GCC with a differential oracle.

**Implementation.** Added test-only Γ-GCC lowering for lifecycle roots/`KeepsAlive` edges.

**Falsification.** Exhaustive root/edge enumeration over all directed three-node lifecycle graphs matches `LifecycleGraph::live_entities` exactly. Existing maintained-lifecycle mutation tests remain green.

**Result.** Lifecycle is now semantically aligned with Γ-GCC without forcing one physical layout/algorithm.

## CLOSED exactly in Pass79

1. no shared production primitive for finite grounded least closure across APNF/fixpoint/lifecycle semantics;
2. APNF determinant closure duplicated a private hyperrule worklist;
3. `kernel-fixpoint` used a separate semantic reachability solver rather than the common grounded-closure law;
4. recursive deletion had no reusable selected-witness local re-fixpoint substrate in mainline;
5. lifecycle/Γ-GCC semantic equivalence was not protected by a mainline parity oracle.

These are concrete substrate closures. They do **not** close the historical general multiway/recursion item or complete lifecycle migration.

## Advanced but NOT CLOSED

1. 🟨 **Positive recursive queries.** Γ-GCC is now available, but recursive `RelExpr`/rule-body lowering through Γ-APNF/SAMF is not production.
2. 🟨 **Lifecycle maintenance unification.** Semantics are parity-pinned; `MaintainedDenseLifecycle` still has a specialized update algorithm rather than using GCC witness-cone maintenance directly.
3. 🟨 **Grounded proof integration.** GCC has a checker/certificate, but determinant/APNF public proof objects do not yet expose the generic grounded certificate as a first-class proof-carrying artifact.
4. 🟨 **Recursive carrier law.** Only finite/idempotent Boolean support is admitted by this substrate. Unrestricted Bag/Natural recursive multiplicity remains explicitly unsupported.
5. 🟨 **General multiway mathematics.** Γ-OMC/APNF/SAMF/GCC foundations are converging, but final residual/no-enumeration executor mathematics remains in R&D.

## Historical OPEN accounting

Pass78 ended with **22 active historical OPEN**.

- Historical OPEN fully closed in Pass79: **0 / 22**;
- genuinely new historical OPEN: **0**;
- total active OPEN after Pass79: **22**.

The active historical frontier remains:

1. structural/custom semantic physical persistence and structural ordering;
2. general nested/multiway/bushy execution and recursive residual execution;
3. unified cross-family/observable physical lifecycle and advisor;
4. autonomous workload telemetry, correlations, decay/hysteresis and scheduling;
5. exact resident/shared memory pressure accounting;
6. remaining physical layouts plus OrderedView/pagination;
7. recovery rebuild economics beyond deterministic admission/continuation;
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
- `cargo test --workspace --all-targets` — **490 passed / 0 failed / 8 ignored**;
- `cargo clippy --workspace --all-targets -- -D warnings`;
- exact warmed `cargo test --workspace --all-targets --release` — **490 / 0 / 8**;
- `cargo build --workspace --release`;
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps`;
- exact warmed `RUSTFLAGS='-C overflow-checks=yes' cargo test --workspace --all-targets --release` — **490 / 0 / 8**;
- source freeze/post-gate SHA inventories are byte-identical.

Cold monolithic release/overflow compilation hit the tool-call timeout. All 22 crates were therefore warmed and verified in package batches; the exact workspace commands were then repeated successfully on the same targets.

Snapshot:

- **498 declared tests**;
- **22 workspace crates**;
- `kernel-grounded-closure`: 6 tests;
- `kernel-fixpoint`: 5 tests;
- `kernel-lifecycle`: 12 tests;
- `kernel-semantics`: 54 tests;
- **70,342 Rust LOC**;
- **0 external registry/git Cargo packages**;
- **0 unsafe tokens**;
- **19 existing `#[allow(...)]` attributes**, none introduced for this pass;
- **0 TODO/FIXME/todo!/unimplemented!**.

## Next logical cluster

Do not yet expose unrestricted recursion. The next safe GCC migration layer is one of:

1. exercise witness-cone incremental maintenance against `MaintainedDenseLifecycle` with long randomized/crossover tests, then decide whether GCC maintenance can replace only the semantic update core while preserving dense storage; or
2. wait for the current R&D closeout and use Γ-APNF/SAMF to define the finite semantic atoms/rule instances for positive recursive query support.

The current mainline no longer needs another independent fixed-point semantics.
