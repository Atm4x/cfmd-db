# PASS84 REPORT — DURABLE SEMANTIC-CORE RECOVERY

Status: **VERIFIED / SOURCE FROZEN**.
Baseline: frozen Pass83 `cfmd_workspace_pass83_advisor_recovery_economics.zip` (`fa4c8ba9d664ccc06f8b202eafd1f6f9c38aa0d624f7a91edebeb02c0db9f7e9`).

Wall-clock integration start: **2026-09-22 19:38:35 UTC**.
Source freeze: **2026-09-22 19:58:42 UTC** (20m07s). No production Rust source was edited after freeze.

## Goal and result

Pass84 finishes the sole remaining production remainder of historical #7 without reopening Pass81 write authority: persist a reconstructible semantic core for `ObservableAtom`, replay that core across the exact committed WAL tail under pinned Γ semantics, rehydrate it against fresh runtime row handles, and fail open to an exact rebuild when the core cannot be trusted.

Historical **#7 is now PROD CLOSED**.

## Integrated contract

- `DurableArtifactCore::ObservableAtom` stores `(source_revision, relation, key_parts, canonical keys by durable occurrence ordinal)` and contains no `PhysicalRowId`.
- durable metadata codec advances to **v11**; pre-v11 metadata remains readable and simply has no artifact cores.
- runtime create/checkpoint/materialization-reconfiguration paths persist the derived core together with the existing physical recipe set.
- reopen reconstructs authoritative relation storage first and then rehydrates compatible cores onto fresh runtime handles.
- compatible rehydration does not consume semantic key-rebuild work; normal retained-byte admission still applies.
- WAL-tail replay updates the core through the same pinned-Γ relation equivalence and deterministic first-match removal order as logical replay.
- full revision / schema / semantic-context transitions invalidate the checkpoint core and force the exact rebuild path instead of guessing a transport.
- stale/malformed/incompatible cores are derivative failures only: logical `Revision=(S,Γ,M)` recovery remains authoritative and available.
- Pass83 recovery economics is preserved: initial restart remains telemetry-independent; live telemetry may prioritize later deferred rebuilds, but never becomes durable semantics.

## Hostile coverage

New Pass84 cases cover:

1. checkpoint ObservableAtom rehydration with zero semantic rebuild-work budget;
2. WAL-tail mutation replay before core rehydration;
3. stale-core rejection followed by exact rebuild;
4. ASCII-case-insensitive Bag removal proving the core replay chooses the same first Γ-equivalent occurrence as logical relation replay.

## Verification

Frozen production source delta versus Pass83 is exactly five Rust files:

- `crates/kernel-durability/src/lib.rs`
- `crates/kernel-durability/src/metadata.rs`
- `crates/kernel-durability/src/store.rs`
- `crates/kernel-plan/src/lib.rs`
- `crates/kernel-semantics/src/observable.rs`

Final gate after all source edits:

- `cargo fmt --all -- --check` — PASS
- `cargo check --workspace --all-targets` — PASS
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS
- `cargo test --workspace --all-targets` — PASS
- **628 declared tests**, **8 ignored**
- direct `kernel-plan --lib`: **235 passed / 0 failed / 4 ignored**
- direct `kernel-durability --lib`: **50 passed / 0 failed / 0 ignored**
- no new lint suppression / unsafe / TODO/FIXME/todo!/unimplemented! was introduced by the Pass84 diff
- workspace-local `target/` is absent; Cargo output used `/mnt/data/cfmd_target_pass84`

Frozen Rust-source fingerprint: `1b9ccefd0ac5ce95a06a52e995a2186f603f9f6b1beab202a4d647c49a2d8bf2`.

## Exact next checkpoint

**Pass85: historical #2 — positive recursive Bag execution / PWRC.**

The verified R&D rebase surface was audited during the remaining Pass84 integration window but intentionally not copied after it became clear that it is a genuine three-crate integration rather than a safe tail patch:

- `kernel-fixpoint`: exact arbitrary-precision `BigNatural`, `NaturalInfinity`, grounded productive-SCC classification and positive Bag proof-tree multiplicity;
- `kernel-query`: `PositiveRecursiveRowAtom` / `PositiveRecursiveRowRule` / `FixpointCall`, compact `(row, N∞)` result and explicit `NonFiniteRecursiveMultiplicity` for consumers demanding a finite Bag;
- `kernel-plan`: Γ-pinned `PreparedPositiveRecursivePlan` and execution/Γ-drift boundary.

Required hostile gates include finite oracle parity, duplicate premise multiplicity, productive versus ungrounded cycles, a dead conjunctive premise behind an apparent cycle, huge finite multiplicities, non-recursive-call-stack execution over long carriers, and Γ drift. Do not mechanically copy the Pass80 workspace; rebase those contracts onto current Pass84 query/plan authority.
