# PASS28 REPORT — joint prepared storage + maintained-plan revision transition

Status: **SOURCE-AUDITED / NOT COMPILED IN THIS CONTAINER**.

Source wall-clock: **2026-09-19 19:54:31 → 20:14:32 UTC = 20m01s**. Production source was frozen at the boundary; everything after it is audit, reporting and packaging only.

## 1. Problem

Pass27 removed duplicate semantic membership lookup at the storage→maintained-plan leaf boundary, but left a more fundamental publication window:

```text
PhysicalStore COMMITTED
maintained plan NOT YET COMMITTED
```

`PhysicalStore::apply_relation_delta_certified` produced its certificate only after mutating authoritative storage. `MaterializedRelPlanState::apply_storage_certified_deltas` then independently mutated its recursive derived state. A failure or stale consumer between those operations could therefore expose different logical revisions in the two live structures.

A second defect was identity: neither live structure was explicitly bound to a logical `RevisionId`, and a stable row handle alone (`slot + generation`) is not a revision identity. A prepared transition also needs freshness strong enough to reject a different state that merely happens to have the same revision/epoch counters.

## 2. Hypothesis

Make preparation purely functional with respect to live state:

```text
source live state
    ↓ clone / validate / mutate candidate only
PreparedPhysicalStoreTransition
    +
PreparedMaterializedRelPlanTransition
    ↓ joint freshness validation
extract BOTH candidates without touching live state
    ↓
no further fallible Result-producing work
    ↓
replace storage live state
replace maintained-plan live state
```

Bind both participants to `source_revision`, carry `target_revision` in the candidate, and retain an exact source snapshot in this correctness-first pass. Epochs reject ordinary stale transitions; exact source-state equality also rejects ABA-like/same-counter-but-different-state cases.

The full-source clone is intentionally not the final performance design. It is a falsifiable reference contract that can later be replaced by a collision-resistant state/version identity or COW snapshot once correctness is verified.

## 3. Implementation

Only two production source files changed:

- `crates/kernel-plan/src/lib.rs`
- `crates/kernel-query/src/lib.rs`

### 3.1 PhysicalStore prepared transition

`PhysicalStore` now carries:

- `transition_epoch: u64`;
- `revision: Option<RevisionId>`.

Added private `PreparedPhysicalStoreTransition` containing:

- source epoch;
- source revision;
- exact source snapshot;
- fully mutated candidate store;
- `StorageCertifiedRelationDelta` produced from that candidate.

The storage-side prepared object is intentionally private to `kernel-plan`; callers cannot obtain it and publish authoritative storage independently through the new API.

`PhysicalStore::bind_revision` binds an initialized snapshot once. When bound, legacy semantic mutation through `install`, `apply_relation_delta`, and `apply_relation_delta_certified` is rejected with `RevisionBoundMutationRequiresPreparedTransition`.

`install_i64_index` remains legal because the persisted index is reconstructible physical state, not semantic authority. It advances the epoch, so every already-prepared semantic transition becomes stale rather than overwriting the newer physical state.

All epoch progression uses `checked_add`; wraparound is rejected as `TransitionEpochExhausted` rather than creating an ABA window.

### 3.2 Maintained-plan prepared transition

`MaterializedRelPlanState` now also carries:

- `transition_epoch: u64`;
- `revision: Option<RevisionId>`.

Added `PreparedMaterializedRelPlanTransition` with the same correctness-first source/candidate structure plus its output `RelationDelta`.

Revision-aware preparation applies storage-certified leaf deltas and recursive Filter/Project/Join/Distinct/Group/TopK propagation only to the candidate. Live maintained state is untouched during prepare.

Legacy `apply_relation_deltas` and `apply_storage_certified_deltas` now use candidate→validate→swap for unbound compatibility mode and are rejected for revision-bound state.

`attach_storage_handles` was also changed to candidate→swap. Previously, a recursive Join could mutate one child and then fail in the other child, leaving a partially attached bootstrap snapshot. This path is physical metadata rather than semantic mutation, so it remains legal after revision binding but advances the maintained-plan epoch and invalidates stale prepared transitions.

### 3.3 Joint public boundary

Added:

```text
prepare_storage_plan_transition(...)
    -> PreparedStoragePlanTransition

PreparedStoragePlanTransition::commit(&mut PhysicalStore,
                                      &mut MaterializedRelPlanState)
```

Preparation requires both participants to be bound to the same `source_revision`, rejects `source_revision == target_revision`, prepares authoritative storage first without publishing it, converts the resulting exact handle receipt into the maintained-plan candidate, and returns one joint object.

Commit consumes that joint object. It first validates/extracts the storage candidate, then validates/extracts the plan candidate; neither extraction mutates live state. Only if both checks succeed are the two live values replaced. No `Result`-producing/fallible operation occurs after the first live assignment.

Within ordinary safe in-memory execution, the method holds exclusive `&mut` access to both participants for the whole commit call, so outside safe readers cannot interleave between the two replacements. This does **not** claim crash durability: process/filesystem failure between replacements is an Agent-1 WAL/recovery problem.

## 4. Hostile falsification added

Ten source tests were added in `kernel-plan`:

1. `joint_prepare_is_invisible_until_commit_and_then_publishes_both_states`
2. `failed_plan_prepare_does_not_publish_prepared_storage_candidate`
3. `stale_joint_transition_is_rejected_before_either_participant_is_swapped`
4. `revision_mismatch_during_joint_prepare_leaves_both_live_states_unchanged`
5. `storage_epoch_change_makes_joint_transition_stale_before_plan_swap`
6. `competing_prepared_transitions_from_same_revision_cannot_both_commit`
7. `sequential_joint_transitions_carry_revision_and_stable_handles_forward`
8. `joint_prepare_rejects_same_source_and_target_revision_without_mutation`
9. `prepared_storage_transition_rejects_different_same_epoch_same_revision_store`
10. `revision_bound_states_reject_legacy_semantic_mutation_entrypoints`

The last test also covers `PhysicalStore::install` after revision binding, closing a late source-audit bypass found before freeze.

These are **declared tests only in this container**. They have not been executed because no Rust toolchain is installed.

## 5. Static audit after freeze

Frozen production-source SHA-256:

- `crates/kernel-plan/src/lib.rs` — `dace2013c9873de2e7a23be69443b087e41e098ee25f1f43121bbdc0b646f4c6`
- `crates/kernel-query/src/lib.rs` — `5b93c8fa7edd974a0f42baa43000107ac9ab05a7537b9ea6c52f53a8a47c8eae`

Static facts:

- 235 declared tests (Pass27: 225);
- 29,140 Rust LOC;
- 19 workspace crates;
- zero external Cargo sources;
- zero `unsafe` blocks;
- zero TODO/FIXME;
- zero `panic!` / `todo!` / `unimplemented!` macros;
- no tabs or trailing whitespace in either changed source file;
- raw delimiter counts balance;
- custom comment/string-aware delimiter scanner passes both changed files;
- Pass27→Pass28 production-source diff is exactly the two files listed above.

Toolchain evidence is in `evidence/pass28/PASS28_TOOLCHAIN_EVIDENCE.txt`:

```text
cargo: command not found
rustc: command not found
rustfmt: command not found
clippy-driver: command not found
```

Therefore **fmt/test/clippy/build/rustdoc have not been run and no VERIFIED claim is made**.

## 6. What Pass28 closes at source-contract level

- [x] prepare no longer needs to publish authoritative storage before maintained-plan validation;
- [x] one joint object owns both prepared candidates;
- [x] source and target logical `RevisionId` are explicit;
- [x] stale storage or maintained state rejects commit before either live state is replaced;
- [x] competing transitions prepared from one source revision cannot both commit sequentially;
- [x] exact source snapshot rejects a different state with the same epoch/revision counters;
- [x] bound legacy semantic mutation entrypoints are blocked;
- [x] revision/epoch advancement is overflow-checked;
- [x] recursive storage-handle attachment no longer partially mutates on error.

## 7. Still OPEN

### Immediate verification gate

- [ ] compile Pass28 with Rust 1.98.1;
- [ ] `cargo fmt --all -- --check`;
- [ ] workspace debug/release tests;
- [ ] strict Clippy;
- [ ] release build;
- [ ] strict rustdoc;
- [ ] overflow-checked release tests.

Until this gate passes, Pass27 remains the last fully verified checkpoint.

### Central transaction architecture

- [ ] **multi-relation / batch revision**: current joint prepared transition carries one relation delta; a real revision may need several base relations to advance atomically;
- [ ] integrate revision ownership with the actual revision/runtime transaction owner instead of trusting caller-supplied `RevisionId` binding;
- [ ] decide final state-identity mechanism. Full source snapshots are correctness-first and too expensive for production;
- [ ] cross-crate capability sealing: `PreparedMaterializedRelPlanTransition::into_candidate_if_current` must currently be public because `kernel-plan` consumes it;
- [ ] `StorageCertifiedRelationDelta::new` is still publicly constructible, so Pass27 certificate-authority sealing remains open;
- [ ] target-revision schema/Γ transition is not solved here. This pass assumes the maintained query's pinned `SemanticContext` remains valid for the prepared delta.

### Durability / Agent 1 boundary

- [ ] process/filesystem crash atomicity;
- [ ] WAL prepare/commit markers, durable revision identity, replay and checkpoints;
- [ ] recovery semantics if a process dies between the two in-memory live replacements.

Runtime epoch and exact in-memory source snapshots are freshness evidence, **not durable authority**.

### Semantic-index / Agent 2 boundary

Future generic semantic indexes remain reconstructible physical state. Any physical index mutation must participate in freshness invalidation, and an index built for one pinned Γ must never silently survive a Γ mismatch.

## 8. Result

Pass28 establishes a concrete two-phase source contract for the Pass27 atomic-publication defect: both physical storage and recursive maintained-plan state can now be fully prepared and freshness-validated before either live state is replaced, with explicit source→target revision binding and hostile stale-transition cases represented in tests.

The design is deliberately conservative: full-state candidate clones and exact source snapshots favor proof of semantics over performance. The next local orchestrator step after external verification is to generalize this contract from one relation to a batch/revision transaction and then choose a cheaper immutable/COW state identity without weakening the falsifiers.
