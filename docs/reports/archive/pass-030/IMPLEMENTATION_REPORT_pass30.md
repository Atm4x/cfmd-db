# CFMD implementation report — Pass30

Status: **VERIFIED** on Rust **1.98.1**.

## Executive result

Pass30 promotes Pass29's runtime transaction root into an authoritative revision publication root.

The live object now owns:

```text
validated kernel_revision::Revision = (S, Γ, M)
selected authoritative PhysicalStore relation layouts
MaterializationId -> MaterializedRelPlanState registry
```

A revision candidate may be published only if the declared logical relation deltas exactly transform the source revision state into the supplied validated target revision state. Physical state and every maintained consumer are derived/prepared under the same transition and published by the same infallible whole-root replacement.

## Production changes

### `kernel-types`

Added nominal `MaterializationId`.

### `kernel-query`

Exposed the exact semantic helpers needed by the authority layer rather than reimplementing query semantics in `kernel-plan`:

- `RelationDelta::apply_to_value`;
- `RelationDelta::between_values`;
- `RelationValue::into_rows`;
- `MaterializedRelPlanState::scan_relations`.

These remain semantic operations backed by the pinned `SemanticContext`/registry.

### `kernel-plan`

`RuntimeRevisionBundle` now owns the complete `Revision`, physical store, selected relation-layout registry and materialization registry.

Construction is checked through `RuntimeRevisionBundle::build`; arbitrary externally prepared maintained state is no longer an accepted constructor input.

`RevisionTransitionRequest` now supplies a full validated target `Revision`. `prepare_revision` proves that the normalized relation-delta descriptor produces exactly that target logical state before it prepares physical and maintained derivatives.

`RevisionCommitDescriptor` carries logical source/target identity plus exact semantic relation deltas across the sealed boundary. It intentionally excludes physical handles and layout mutations from durable authority.

## Authority law

```text
Revision@R
+ physical@R
+ materializations@R
      |
      | prepare exact logical change and runtime derivatives
      v
PreparedRuntimeRevisionTransition
      |
      | exact source freshness
      v
SealedRuntimeRevisionTransition
      |
      | future WAL durability barrier
      v
publish() // no Result, no semantic work
      |
      v
Revision@R' + physical@R' + materializations@R'
```

The source semantic context is authoritative. Pass30 data-plane mutation intentionally rejects a target with a changed `SemanticContext`; schema/Γ migration remains explicit OPEN work.

## Bootstrap law

The builder proves a coherent initial runtime root:

1. selected relation-layout keys must equal schema relation ids;
2. selected physical relation contents must be semantically equal to the authoritative revision model;
3. physical store is bound only after that check;
4. each materialization is rebuilt from the authoritative model/context;
5. storage handles are attached for every scanned base relation;
6. each materialization is revision-bound to the same revision.

This closes the Pass29 constructor trust gap.

## Multi-materialization law

All registered materializations are part of the same candidate even when only some depend on changed base relations. An unaffected materialization may return an empty `RelationDelta`, but it still moves to the target revision with the whole bundle.

Duplicate `MaterializationId` is rejected during bootstrap.

## New hostile tests

1. logical/physical bootstrap divergence is rejected;
2. target Revision state/declared mutation mismatch is rejected;
3. multiple materializations advance atomically under one revision;
4. duplicate materialization identity is rejected;
5. all previous Pass29 prepare/seal/publish stale and rollback falsifiers remain green.

`kernel-plan`: **59 tests**.

## Full verification

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

Static state:

```text
19 crates
241 declared tests
59 kernel-plan lib tests
29,818 Rust LOC
0 external Cargo source entries
0 unsafe occurrences under crates/
```

Evidence: `evidence/pass30/`.

## Чеклист Pass30 — CLOSED / OPEN

### Закрыто именно в этом pass

1. [x] Numeric-only runtime revision authority — **CLOSED**; bundle owns actual validated `Revision`.
2. [x] External logical/runtime publication owners — **CLOSED for current data-plane path**.
3. [x] Caller-trusted maintained bootstrap snapshot — **CLOSED**.
4. [x] Unverified logical↔physical bootstrap coherence — **CLOSED**.
5. [x] Single maintained consumer owner — **CLOSED** via `MaterializationId` registry.
6. [x] Target-id-only transition — **CLOSED**; target is a complete validated Revision.
7. [x] Target logical state may disagree with declared deltas — **CLOSED** by exact validation.
8. [x] Per-mutation caller-selected physical layout — **CLOSED**; runtime relation binding owns the choice.
9. [x] Missing logical descriptor at sealed durability boundary — **CLOSED for unchanged-Γ relation-data revisions**.
10. [x] Materialization registry partial revision advancement — **CLOSED** by whole-candidate publication.

### Закрыто из OPEN-чеклиста Pass29: **4 / 11**

1. [x] Full authoritative Revision ownership.
2. [x] Sealed logical relation-mutation descriptor.
3. [x] Multi-materialization registry.
4. [x] Checked logical/physical/maintained bootstrap.

### Осталось из прошлого OPEN: **7 / 11**

1. [ ] Immutable/COW/version-root source identity instead of full source/candidate clones.
2. [ ] Cross-crate prepared-plan capability sealing.
3. [ ] Storage-certified receipt minting authority sealing.
4. [ ] Schema/Γ-changing revision migration transaction.
5. [ ] Process/filesystem crash atomicity.
6. [ ] Production WAL/replay/checkpoint/segment recovery.
7. [ ] Concrete reader publication primitive / MVCC synchronization.

### Новые / уточнённые OPEN Pass30

1. [ ] Exactly one selected base relation layout per schema relation is currently part of the authoritative runtime root; alternate reconstructible layouts need a separate derived registry.
2. [ ] Relation-only unchanged-Γ descriptor must later generalize to lifecycle/field/schema/semantic revision changes or coexist with typed migration transaction classes.
3. [ ] Durable encoding/versioning/checksum of `RevisionCommitDescriptor` is not yet production code.
4. [ ] Bootstrap semantic coherence verification needs a cheaper certified fast path for large states.
5. [ ] Full candidate/source clones remain an intentionally expensive correctness reference implementation.

### Отложенный project-wide OPEN

1. [ ] Agent-2 semantic-index integration.
2. [ ] Generic Text/F64/Bool/entity indexes and generic maintained TopK order-statistics.
3. [ ] Stateful typed-batch outputs and remaining constant-factor gaps.
4. [ ] Remaining physical layouts / OrderedView / pagination.
5. [ ] Transaction repair, distribution/replication, formal closure.

## Next implementation target

One more central cleanup before durability integration is preferable:

1. seal storage-certified and prepared-plan construction capabilities;
2. introduce immutable/versioned publication identity so freshness no longer requires full bundle snapshots;
3. define the reader-visible root primitive.

After that, integrate the already verified Agent-1 logical WAL protocol at the existing sealed seam. Agent-2 indexes remain intentionally deferred until the durable revision pipeline is stable.
