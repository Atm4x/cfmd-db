# PASS74 REPORT — cyclic prefix indexing + Program9 durable typed-layout recovery

Status: **VERIFIED**.

Source freeze: **2026-09-21 11:48:31 UTC** (refrozen after removing a patch backup file; production bytes unchanged).

Production diff relative to Pass73:

- `crates/kernel-durability/src/lib.rs`
- `crates/kernel-durability/src/metadata.rs`
- `crates/kernel-plan/src/lib.rs`

## Problem → hypothesis → implementation → falsification → result

### 1. Bounded cyclic Γ-QCN still scanned broad ordinal domains

**Problem.** Pass73 safely admitted bounded non-GYO/cyclic Γ-QCN, but its DFS could still scan wide physical ordinal ranges even when already-assigned quotient constraints implied a sparse joint candidate set.

**Hypothesis.** For cyclic search orders, pre-index each later leaf by the joint canonical-key signature of constraints whose opposite endpoint is already assigned. This can narrow candidate iteration without changing Γ semantics or output order.

**Implementation.** Pass74 adds `CyclicPrefixCandidateIndexes`. Active ordinals are grouped by deterministic joint `CanonicalEqKey` signatures, lookup work is included in the bounded-cyclic work certificate, and enumeration visits only indexed candidates for prefixes where such an index is available. The executor still validates all original quotient constraints before accepting an assignment. Dedicated telemetry records prefix-index lookups and semantic-quotient candidate visits.

**Falsification.** Sparse non-GYO fixtures require exact reference equality while proving candidate visits are far below physical scanned rows. Joint-constraint hostiles verify that the prefix index tightens domains only when all relevant assigned-key coordinates agree. Malformed orders and over-budget cases remain fail-closed.

**Result.** The bounded-cyclic branch is no longer forced to pay a full ordinal scan at every prefix when exact joint quotient support is available.

### 2. Program9: native physical representation disappeared across reopen

**Problem.** Pass73 logical recovery was exact, but supported typed/columnar native layouts still reopened through generic `RECOVERY_ROW_STORE`; exact persisted I64 index state therefore could not be reconstructed as a durable derived optimization.

**Hypothesis.** Persist only versioned logical lowering recipes and rebuild fresh physical state from recovered `Revision=(S,Γ,M)`. Never persist revision-local physical handles or dense IDs.

**Implementation.** Program9 is integrated with recipe format v2. Durable specs now include `RelationLayout` (`RowStore`, `ValueColumnar`, `I64Columnar`, `TypedColumnar`) and `I64Index`. Recovery lowers authoritative rows into a fresh native relation; typed live references bind to the recovered Revision's fresh dense identity table; I64 index state is rebuilt afterward under pinned Γ. Recipe v1 remains decodable.

**Falsification.** Typed-layout + I64 index checkpoint/compaction/reopen serves a subsequent Join with `persisted_index_hits=1` and no transient index build. Row/value/I64 layout round trips pass. LiveRef recovery proves a fresh dense table maps to the same external identities. Stale or contradictory recipes fall back deterministically without blocking logical recovery; unknown recipe versions fail closed.

**Result.** Supported native layouts and exact I64 indexes are reconstructible across reopen without promoting physical identity to durable authority.

### 3. Cross-program compatibility

**Problem.** Program9 was authored on Pass72, while authoritative production already contained Pass73 bounded cyclic planning and in-progress Pass74 cyclic prefix indexing.

**Hypothesis.** The changes are orthogonal if Program9 modifies only physical recovery recipes and does not alter Pass73/74 QCN semantic laws.

**Implementation.** Program9 package SHA is verified. Its patch applies to Pass74 with `--fuzz=0`; no fuzzy/context merge is used. Pass73/74 cyclic tests and Program9 recovery tests are rerun on the merged source.

**Falsification.** Full workspace debug/release/overflow gates pass; freeze/post-gate SHA inventories are byte-identical.

**Result.** Program9 is accepted into Pass74 without regressing the bounded-cyclic planner.

## CLOSED exactly in Pass74 — narrower production defects

1. cyclic QCN prefixes repeatedly scanning broad ordinal domains despite exact assigned quotient signatures;
2. supported typed/columnar relation layouts being lost on recovery;
3. exact I64 indexes having no durable logical rebuild recipe;
4. typed LiveRef layout recovery lacking an explicit fresh dense-ID rebinding contract;
5. contradictory/stale physical layout advice lacking the Program9 deterministic recovery law.

## Historical OPEN accounting

Pass73 ended with **22 active historical OPEN**.

- Historical OPEN fully closed in Pass74: **0 / 22**;
- genuinely new OPEN: **0**;
- total active OPEN after Pass74: **22**.

The historical recovery item remains broader than Program9: rebuild economics/advisor policy, arbitrary future layout families, generic format migration, streaming checkpoints and physical power-loss proof remain OPEN. The general multiway item also remains OPEN because bounded cyclic/prefix-index execution is not a worst-case-optimal/general hypertree-width planner.

## Verification

Rust: `rustc 1.98.1 (48a229cea 2026-09-01)`.

Frozen-source final gate PASS:

- `cargo fmt --all -- --check`;
- `cargo check --workspace --all-targets`;
- `cargo test --workspace --all-targets` — **450 passed / 0 failed / 8 ignored**;
- `cargo clippy --workspace --all-targets -- -D warnings`;
- `cargo test --workspace --all-targets --release` — **450 / 0 / 8**;
- `cargo build --workspace --release`;
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps`;
- `RUSTFLAGS='-C overflow-checks=yes' cargo test --workspace --all-targets --release` — **450 / 0 / 8**.

Cold release/overflow attempts that exceeded external command windows were not counted. After warming the affected release fingerprint, the full workspace invocations completed successfully with exit code 0.

Freeze/post-gate `crates/` SHA-256 inventories are byte-identical.

Frozen snapshot:

- **458 declared tests**;
- **189 `kernel-plan` declared tests**;
- **21 crates**;
- **64,701 Rust LOC**;
- **0 external registry/git Cargo sources**;
- **0 unsafe hits**;
- **19 existing `#[allow(...)]`**, no new suppression;
- **0 TODO/FIXME/todo!/unimplemented! hits**.

## Next frontier

The most direct remaining recovery work is reconstruction economics/policy for future physical families rather than more authority mechanisms. On the planner side, bounded cyclic search is now both work-certified and prefix-indexed, but unrestricted/high-width cyclic joins still need a stronger general strategy before the historical multiway item can close.
