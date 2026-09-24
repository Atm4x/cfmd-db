# IMPLEMENTATION REPORT — Pass88

## Scope

Pass88 rebased two verified R&D closures onto the frozen Pass87 production tree without replacing late Pass81–87 architecture: historical #19 bounded repair and historical #9 canonical durable-format migration.

## Changed production files

### `crates/kernel-plan/Cargo.toml`

Adds the existing `kernel-transport` crate as an explicit dependency for verified cross-context repair comparison.

### `crates/kernel-plan/src/lib.rs`

Adds the bounded repair production surface:

- `RuntimeRepairRelationMutation`;
- `RuntimeRepairCandidate` for relation-local, full-revision and transported full-revision candidates;
- `RuntimeRepairObservationTransport` backed by verified transport witnesses;
- finite `RepairCandidateProvider` plus `Vec` implementation;
- `RepairSearchPolicy`, `RepairSearchReport`, `RepairSearchOutcome`;
- transported observation impact evaluation;
- `prepare_repair_candidate` and `prepare_bounded_repair`;
- typed transport/mismatch errors;
- hostile tests for bounds, uniqueness/ambiguity, VMF/OFC and transport behavior.

The implementation does not make repair authoritative. Every accepted result is still a normal prepared transition under the existing Revision/Γ/VMF/OFC publication spine.

### `crates/kernel-query/src/lib.rs`

Adds a read-only accessor for the `RelObservationGuard` query so the existing transport layer can prove cross-context observation equivalence. No query semantics changed.

### `crates/kernel-durability/src/lib.rs`

Adds `DurableFormatComponent` and typed `DurabilityError::UnsupportedDurableFormat` so unsupported published authority is distinguishable from generic corruption and cannot trigger silent rollback behavior.

### `crates/kernel-durability/src/store.rs`

Adds:

- `DurableFormatRegistry` support checks for immutable generation components;
- explicit `CanonicalDurableState` recovered-authority assembly;
- runtime-store publication from canonical state only after reconciliation;
- `migrate_to_current_format`, which writes a fresh current-format generation from recovered authority;
- hostile no-fallback test for an unsupported highest manifest;
- real historical metadata-v6 -> canonical -> fresh current generation migration test.

WAL remains independently versioned and decoded before canonical publication; no physical codec was made semantic authority.

### `Cargo.lock`

Updated only for the new `kernel-plan -> kernel-transport` workspace dependency edge.

## Verification

Final combined gate on the frozen production tree:

- `cargo fmt --all -- --check` — PASS;
- `cargo check --workspace --all-targets` — PASS;
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS;
- `cargo test --workspace --all-targets` — PASS;
- declared tests: **660**;
- failed: **0**;
- ignored: **8**.

Frozen source fingerprint over all `crates/**/*.rs` and crate `Cargo.toml` files: `11aaee6a4909aadc200b14b8cd8f556efa2002af012781f7bdc10dec3170af95`.

## Deliberately not implemented

Pass88 does not claim #12 streaming/chunked checkpoints, #13 real platform power-cut assurance, #17 authenticated durable storage, or #18 formal publication/fsync/GC mechanization. Those remain separate whole historical rows.
