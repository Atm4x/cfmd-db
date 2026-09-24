# CFMD implementation report — Pass29

Status: **VERIFIED**.

## Executive result

Pass29 turns the Pass28 one-relation joint transition into a correctness-first **multi-relation runtime revision transaction** with an explicit pre-durable sealing boundary.

The central runtime law is now:

```text
prepare candidates only
        -> seal exact source freshness while holding exclusive live borrow
        -> [future WAL COMMIT/fsync]
        -> infallible whole-bundle publish
```

No semantic validation is deferred beyond `seal()`.

## Baseline correction

Before Pass29, the supplied Rust 1.98.1 toolchain was installed and the original Pass28 source-audited branch was reopened. Real compiler/test failures were fixed rather than waived. The corrected Pass28 passed the complete workspace gate and became the verified base for Pass29.

See `PASS28_VERIFICATION_REPORT.md` for the corrected historical status.

## New production abstractions

### `RuntimeRevisionBundle`

One owner now contains:

```text
RevisionId
PhysicalStore
MaterializedRelPlanState
```

Only immutable child access is exposed. Reconstructible physical-index installation is routed through the owner and changes freshness.

### `RevisionRelationMutation`

One normalized mutation of one semantic base relation:

```text
relation + physical layout + exact RelationDelta
```

### `RevisionTransitionRequest`

Contains:

```text
target RevisionId
[] RevisionRelationMutation
pinned SemanticContext
SemanticRegistry
```

The source revision is read from the live bundle, not supplied by the caller.

### `PreparedRuntimeRevisionTransition`

Owns:

- exact source bundle snapshot;
- complete target bundle candidate;
- source and target revision ids;
- already computed root output delta.

Preparation may fail freely; live state remains untouched.

### `SealedRuntimeRevisionTransition<'live>`

Created only after exact live/source equality succeeds. It retains `&mut RuntimeRevisionBundle` until `publish` or drop.

This type is intentionally the future durability handoff. A WAL integration may perform its append/fsync while holding the sealed capability; no safe-Rust mutation of the live bundle can race between the final freshness test and publication.

`publish(self)` is semantically infallible and replaces the complete bundle.

## Batch semantics

Pass29 accepts several base-relation mutations in one revision.

Rules:

1. target revision must differ from source;
2. each semantic relation appears at most once in the normalized batch;
3. every physical mutation is validated/applied to one cloned candidate store;
4. exact storage receipts for all changed relations are collected;
5. the maintained recursive tree consumes the complete receipt map as one candidate transition;
6. any failure discards the candidate;
7. publication changes the whole runtime bundle together.

The hostile two-relation Join case verifies that simultaneous mutations of both Join inputs produce the same final maintained relation as a fresh logical recomputation.

## Removed architecture

Pass28's public:

```text
StoragePlanTransitionRequest
PreparedStoragePlanTransition
prepare_storage_plan_transition
```

was removed. Keeping both old and new transaction protocols would make it possible for future durability code to bind to the wrong authority boundary.

The private legacy unbound physical prepared object remains only for compatibility before a `PhysicalStore` is revision-bound.

## Correctness properties established

1. prepare cannot mutate live runtime state;
2. a batch failure on relation N cannot expose relations 1..N-1;
3. exact source snapshot prevents same-revision/same-counter substitution;
4. reconstructible physical mutation after prepare invalidates sealing;
5. two candidates from one source cannot both publish sequentially;
6. stable row-handle generation survives sequential revision publication;
7. dropping a seal is an abort;
8. after successful seal there is no transaction `Result` path in publication;
9. physical store, maintained state and bundle revision id advance together inside the runtime owner.

## Verification

Rust 1.98.1 complete gate: **PASS**.

```text
cargo fmt --all -- --check                                  PASS
cargo check --workspace --all-targets                       PASS
cargo test --workspace --all-targets                        PASS
cargo clippy --workspace --all-targets -- -D warnings       PASS
cargo test --workspace --all-targets --release              PASS
cargo build --workspace --release                           PASS
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps  PASS
RUSTFLAGS='-C overflow-checks=yes' cargo test ... --release PASS
```

Evidence: `evidence/pass29/`.

Workspace state:

```text
19 crates
237 declared tests
55 kernel-plan lib tests
29,302 Rust LOC
0 external Cargo source entries
0 unsafe blocks
```

## Чеклист Pass29 — CLOSED / OPEN

### Закрыто именно в этом pass

1. [x] Pass28 compiler/test uncertainty — **CLOSED**.
2. [x] Single-relation-only runtime transaction — **CLOSED**.
3. [x] Independent storage/plan live publication API — **CLOSED for the current one-plan owner**.
4. [x] Caller-supplied source revision — **CLOSED**.
5. [x] Freshness failure after the future durability point — **CLOSED by `seal()` contract**.
6. [x] Partial live publication from a later relation failure — **CLOSED**.
7. [x] Sequential double-commit of competing prepared candidates — **CLOSED**.
8. [x] Duplicate old/new public transaction protocols — **CLOSED**.
9. [x] Two-independent-replacement publication window — **CLOSED** by whole-bundle publication.

### Закрыто из OPEN-чеклиста Pass28: **9 / 16**

1. [x] Rust 1.98.1 compilation gate.
2. [x] rustfmt gate.
3. [x] debug/release workspace tests.
4. [x] strict Clippy.
5. [x] release build.
6. [x] strict rustdoc.
7. [x] overflow-checked release tests.
8. [x] multi-relation / batch revision.
9. [x] the specific crash/recovery window caused by two separate live assignments no longer exists; general durability remains open.

### Осталось из прошлого OPEN: **7 / 16**

1. [ ] Authoritative `kernel_revision::Revision` ownership.
2. [ ] Cheaper immutable/COW/version-root source identity.
3. [ ] Cross-crate prepared-plan capability sealing.
4. [ ] `StorageCertifiedRelationDelta` minting authority sealing.
5. [ ] Schema/Γ-changing target revisions.
6. [ ] Process/filesystem crash atomicity.
7. [ ] Production WAL/recovery/checkpoint/segment integration.

### Новые / уточнённые OPEN Pass29

1. [ ] `RuntimeRevisionBundle` must own or uniquely bind the actual validated logical `Revision=(S,Γ,M)`, not just its id.
2. [ ] A complete durable logical mutation descriptor must be carried across the sealed boundary.
3. [ ] Multiple maintained materializations need a registry identity/freshness contract and one all-or-nothing candidate.
4. [ ] Bootstrap construction must prove logical/physical/maintained coherence instead of trusting arbitrary supplied snapshots.
5. [ ] Concrete reader publication/MVCC synchronization remains open.

### Отложенный project-wide OPEN

1. [ ] Agent-2 canonical semantic-index production integration.
2. [ ] Generic Text/F64 maintained TopK/indexed Join/Group.
3. [ ] Group/TopK typed-batch outputs and constant-factor gaps.
4. [ ] Remaining physical layouts and OrderedView/pagination.
5. [ ] Transaction repair, distribution/replication and formal closure.

## Next implementation target

Keep the mainline single-threaded architecturally: finish the authoritative revision owner first.

The next pass should make the publication root own (or otherwise uniquely bind) the actual validated logical `Revision = (S, Γ, M)`, then generalize maintained state from one tree to a registry if required by the intended DB surface. The sealed capability should be the only object accepted by the later durability layer.

After that, integrate Agent-1 WAL. Only once the durable revision pipeline is stable should Agent-2 canonical semantic indexes be moved from research prototype into production Join/Group/TopK paths.
