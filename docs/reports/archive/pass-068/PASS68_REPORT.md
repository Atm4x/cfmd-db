# CFMD Pass68 Report — corrected Program7 integration

Date: 2026-09-21
Status: **VERIFIED**
Authoritative base: verified Pass67
Production source freeze: **2026-09-21 00:52:10 UTC**

## Goal

Hostile-review the corrected R&D Program7 against the exact retry falsifier that rejected its first version, integrate it only if the authority boundary is sound, and preserve every Pass67 planner/physical-state closure.

## Result

Program7 is integrated in Pass68.

The key correction is an explicit split between two durability authority surfaces:

1. **Legacy full-target relation transition** — `DurableRuntime::commit_revision` still accepts a caller-supplied target `Revision`; therefore durable exact retry retains the canonical full target witness.
2. **Delta-authoritative relation transition** — new `DurableRuntime::commit_derived_relation_data` accepts source/target IDs plus typed relation mutations only; the runtime derives the target Revision from the authoritative source and pinned Γ. This surface may therefore retain compact `RelationDataExact` intent without losing client-controlled information.

The previous Program7 failure is not hidden or reinterpreted: the Pass63 counterexample is still valid for the rejected first design. Pass68 integrates a different authority contract.

## Hostile findings

### Pass63 falsifier

Replayed successfully against the corrected branch. After later commits, checkpoint, compaction and reopen, retrying the legacy full-target transaction with the same transaction id / same nominal target id / same relation delta but different target content still produces `TransactionIdConflict`.

### Compact retry

The new derived relation-data API survives the same history/compaction sequence:

- exact retry -> `AlreadyCommitted`;
- changed delta -> `TransactionIdConflict`;
- recovery reconstructs the same logical target.

### Additional Pass68 hostile

Added `derived_relation_commit_matches_authoritative_target_and_rejects_stale_source`.

It verifies that the runtime-derived target equals the authoritative target constructed by the existing relation transition, and that an uncommitted request naming a stale source is rejected before durable authority advances and remains rejected after reopen.

### Freshness

There is no stale-snapshot publication hole. The derived target is subsequently passed through the normal `prepare_revision` path, and `PreparedRuntimeRevisionTransition::seal()` checks current `root_identity` and source `RevisionId` while holding the sole writer-publication guard before durable COMMIT.

## Durable representation

- relation mutation codec: v5;
- metadata codec: v4;
- compact relation-data PREPARE stores canonical relation mutations once;
- `RelationDataExact` retains source/target IDs, pinned semantic revision/modules and exact canonical relation mutations;
- full-target transitions keep canonical full revision bytes;
- historical decoder compatibility remains present for older relation-data formats.

The existing structural size regression requires a 5,000-entity/one-row-delta compact PREPARE and committed transaction entry to each be more than 100x smaller than full target revision encoding. This is a falsifier for accidental snapshot duplication, not a universal compression claim.

## CLOSED exactly in Pass68 — 1 concrete production problem

1. **Snapshot-sized durable idempotency intent for small relation-data transactions.** A production delta-authoritative API now retains exact delta-sized logical request identity without weakening the legacy independently-supplied-target exactness contract.

## Historical OPEN accounting

Pass67 ended with **23 historical active OPEN**.

- Historical 23 fully closed this pass: **0 / 23**.
- Genuinely new OPEN: **0**.
- Total active OPEN after Pass68: **23**.

The durability-space defect is closed, but historical OPEN #12 is broader: transaction intent/outcome **retention + GC** remains unsolved because the ledger still grows with transaction count.

## Advanced but still OPEN

- Transaction intent/outcome retention + GC: relation-data entries are compact, but no exact retry-retention horizon / pruning law exists.
- General durable-format migration framework: v5/v4 compatibility is implemented locally, not a general migration calculus.
- Streaming/chunked checkpoints and metadata.
- Multi-family physical lifecycle, general multiway planning, structural persisted-key/version discipline and all other Pass67 frontier items remain unchanged.

## Active OPEN after Pass68 — 23

1. Structural/custom-equivalence physical indexing: durable recursive-key encoding/versioning, persisted structural indexes, arbitrary/plugin canonical laws and structural ordering.
2. General nested/multiway/bushy Join planning beyond bounded 3–8-leaf Γ-QCN.
3. Complete multi-family physical lifecycle beyond currently managed semantic indexes, Γ-QCN endpoint factors, persisted I64 indexes and conservative direct-Join semantic statistics.
4. Autonomous workload telemetry/read-write rates/decay/hysteresis/lifecycle scheduling.
5. Exact allocator/RSS accounting, external memory pressure and rebuild scheduling.
6. Canonical-key/cache encoding-version migration and compatibility law.
7. Remaining physical layouts plus explicit `OrderedView`/pagination.
8. Secondary-index/alternate-layout rebuild economics and fast physical reconstruction after recovery.
9. Durable revision DAG / branch+merge ancestry and merge replay.
10. General historical durable-format migration framework.
11. Arbitrary/plugin semantic executable artifact packaging/signing/authentication/deployment.
12. Transaction intent/outcome retention + GC. Compact delta-authoritative relation intent is production, but the ledger remains unbounded in transaction count.
13. Streaming/chunked checkpoints and metadata.
14. Real machine power-loss assurance plus Windows/network-FS/FUSE durability semantics.
15. General lock-poison/restart policy.
16. Group commit / async durability.
17. Replication / consensus and broader distribution architecture.
18. Durable-store authentication/MAC; CRC32C is corruption detection only.
19. Formal power-loss proof for rename/fsync/GC protocol.
20. Transaction repair runtime.
21. Formal mechanization of remaining semantic/transport/retention/power-loss obligations.
22. Maintained I64 Group constant-factor gap.
23. Maintained I64 TopK constant-factor gap.

## Verification

Rust: `rustc 1.98.1 (48a229cea 2026-09-01)`.

Frozen-source final gate PASS:

- `cargo fmt --all -- --check`;
- `cargo check --workspace --all-targets`;
- `cargo test --workspace --all-targets`;
- `cargo clippy --workspace --all-targets -- -D warnings`;
- `cargo test --workspace --all-targets --release`;
- `cargo build --workspace --release`;
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps`;
- `RUSTFLAGS='-C overflow-checks=yes' cargo test --workspace --all-targets --release`.

The first cold release and overflow-check invocations exceeded the external command window during compilation and were not counted. Warmed reruns completed successfully.

Freeze/post-gate `crates/` SHA-256 inventories are byte-identical.

Static frozen-source snapshot:

- **425 declared tests**;
- **167 `kernel-plan` tests**;
- **21 crates**;
- **59,817 Rust LOC**;
- **0 external registry/git Cargo sources**;
- **0 `unsafe` hits**;
- **19 existing `#[allow(...)]`**, no new suppression;
- **0 TODO/FIXME/todo!/unimplemented! hits**.

Production source diff relative to Pass67:

- `crates/kernel-durability/src/lib.rs`;
- `crates/kernel-durability/src/metadata.rs`;
- `crates/kernel-durability/src/store.rs`;
- `crates/kernel-plan/src/lib.rs`.

## Next frontier

Do not extend Program7 directly into implicit ledger GC: exact retry retention needs an explicit retention/horizon contract or another proof-preserving identity structure. Return the mainline to either structural persisted-key/version discipline or general multiway/bushy planning unless a separate retention design is researched first.
