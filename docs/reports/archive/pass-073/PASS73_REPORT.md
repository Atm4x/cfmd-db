# PASS73 REPORT — Bounded cyclic Γ-QCN planning

Status: **SOURCE VERIFIED; ready for packaging**.

Source-work window: **2026-09-21 10:12:34 UTC → 10:32:53 UTC (20m19s)**.
Source freeze: **2026-09-21 10:32:53 UTC**.

Production diff relative to Pass72:

- `crates/kernel-plan/src/lib.rs`

## Problem → hypothesis → implementation → falsification → result

### 1. GYO-only admission for >8-leaf Γ-QCN

**Problem.** Pass72 removed the historical eight-leaf ceiling only when the fully-covered quotient hypergraph admitted a GYO reduction. A genuine non-GYO/cyclic quotient graph still fell back even when its exact search was small.

**Hypothesis.** GYO is a performance certificate, not a correctness requirement. A cyclic quotient branch is safe if it has a deterministic search order and a conservative upper bound on the actual work performed by the current ordinal-scan DFS.

**Implementation.** Pass73 adds deterministic cyclic min-fill ordering and `SemanticQuotientSearchCertificate::BoundedCyclic`. The existing GYO path remains `GyoAcyclic`. The cyclic branch executes only when `quotient_enumeration_work_upper_bound(...)` is within the configured work budget.

**Falsification.** A genuine ten-leaf non-GYO cycle using alternating `TextExact` / ASCII-CI constraints is rejected by GYO, admitted by the bounded-cyclic certificate, and matches the logical evaluator exactly in row order and bag multiplicity.

**Result.** GYO is no longer an admission barrier for exact >8-leaf Γ-QCN when cyclic execution has a certified bounded work envelope.

### 2. Exponential / scan-heavy cyclic fallback risk

**Problem.** Removing the GYO guard without a work law would expose an unbounded DFS. Counting only surviving assignments is also unsound as a performance certificate because the current enumerator scans ordinal ranges at each prefix.

**Hypothesis.** Bound the implementation's actual ordinal-scan work, not merely solution count.

**Implementation.** The certificate validates a full search permutation first, then uses saturating `u128` arithmetic over prefix branching and physical row counts at each depth, plus final materialization cost. Over-budget cyclic programs are rejected before DFS.

**Falsification.** Duplicate-heavy cyclic fixtures exceed the one-million-work budget and reject before enumeration. Malformed search orders with duplicate/missing leaves fail as invalid permutations rather than being misreported as budget exhaustion.

**Result.** The non-GYO branch is fail-safe and bounded by a conservative implementation-specific work certificate.

### 3. Maintained support / advisor integration

**Problem.** A one-shot cyclic branch would not be enough if Program8 maintained quotient support or Pass64 factor lifecycle created artifacts for a runtime path that the cyclic work certificate would reject.

**Hypothesis.** Reuse the same admission law in maintained-support execution and advisor preflight.

**Implementation.** Maintained quotient support feeds the same cyclic search certificate after relation replacement. The quotient-factor advisor rejects an over-budget non-GYO path before creating endpoint factors. `PlanExecutionStats` records `multiway_join_cyclic_budget_rejections` separately from generic inapplicability.

**Falsification.** A materialized non-GYO support fixture remains exact after relation delta; an over-budget cyclic advisor hostile creates zero factors.

**Result.** Bounded cyclic planning composes with Program8 support maintenance and Pass64 lifecycle without paying artifact build cost for rejected paths.

## CLOSED exactly in Pass73 — 4 narrower production problems

1. GYO as the only >8-leaf Γ-QCN performance certificate;
2. no deterministic execution order for fully-covered cyclic/non-GYO quotient hypergraphs;
3. no conservative work guard for cyclic quotient DFS;
4. quotient-factor advisor artifact creation for a cyclic path that runtime would reject by budget.

## Historical OPEN accounting

Pass72 ended with **22 active historical OPEN**.

- Historical OPEN fully closed in Pass73: **0 / 22**.
- Genuinely new OPEN: **0**.
- Total active OPEN after Pass73: **22**.

The general multiway/bushy item remains OPEN. Pass73 provides exact bounded cyclic/non-GYO execution, not a worst-case-optimal join, general hypertree-width planner, or unrestricted cyclic-search law.

## Verification

Rust: `rustc 1.98.1 (48a229cea 2026-09-01)`.

Frozen-source final gate PASS:

- `cargo fmt --all -- --check`;
- `cargo check --workspace --all-targets`;
- `cargo test --workspace --all-targets` — **440 passed / 0 failed / 8 ignored**;
- `cargo clippy --workspace --all-targets -- -D warnings`;
- `cargo test --workspace --all-targets --release` — **440 passed / 0 failed / 8 ignored**;
- `cargo build --workspace --release`;
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps`;
- `RUSTFLAGS='-C overflow-checks=yes' cargo test --workspace --all-targets --release` — **440 passed / 0 failed / 8 ignored**.

Cold release/overflow compilation attempts that exceeded external command windows were not counted; completed warmed runs are the recorded gate. Freeze/post-gate `crates/` SHA-256 inventories are byte-identical.

Frozen snapshot:

- **448 declared tests**;
- **181 `kernel-plan` declared tests**;
- **21 crates**;
- **63,463 Rust LOC**;
- **0 external registry/git Cargo sources**;
- **0 unsafe hits**;
- **19 existing `#[allow(...)]`**, no new suppression;
- **0 TODO/FIXME/todo!/unimplemented! hits**.

## Next frontier

The planning frontier is now narrower: fully-covered non-GYO/cyclic Γ-QCN is available when an execution work certificate fits the configured budget. Remaining work is a more general cost/search law for high-width cyclic joins (e.g. hypertree-width / worst-case-optimal strategy), plus the existing durability/layout/lifecycle frontiers.
