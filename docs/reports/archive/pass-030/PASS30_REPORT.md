# PASS30 REPORT — authoritative revision + materialization-registry publication

Status: **VERIFIED** on Rust **1.98.1**.

Pass30 continues directly from the verified Pass29 transaction core. It deliberately does **not** integrate Agent-1 WAL or Agent-2 semantic indexes yet. The purpose of this pass is to finish the central in-memory authority boundary that both later systems must attach to.

## 1. Problem

Pass29 proved the runtime law:

```text
prepare candidates -> seal final freshness -> [future durable point] -> infallible publish
```

but its `RuntimeRevisionBundle` still owned only:

```text
RevisionId + PhysicalStore + one MaterializedRelPlanState
```

That left four central authority gaps:

1. the logical `kernel_revision::Revision=(S, Γ, M)` lived outside the runtime publication owner;
2. bootstrap trusted a caller-supplied maintained snapshot rather than reconstructing it from semantic authority;
3. only one maintained consumer participated in publication;
4. the future durability seam did not carry a logical revision descriptor sufficient to keep physical handles/layouts out of durable authority.

A fifth consistency gap followed from (1): a caller could provide a target revision id that advanced the runtime while the actual logical target state remained external to the transaction.

## 2. Hypothesis

Make one object own the complete coherent in-memory revision:

```text
RuntimeRevisionBundle@R
  = validated Revision(S, Γ, M)
  + authoritative selected PhysicalStore layouts
  + materialization registry
```

Bootstrap must be a checked construction, not a bag of already-built snapshots. A transition request must name the complete validated target `Revision`, while the runtime independently proves that its declared logical relation deltas transform source `M` into exactly target `M'`.

The final law becomes:

```text
Authoritative Revision@R
+ PhysicalStore@R
+ MaterializationRegistry@R
        |
        | exact logical relation deltas
        | + complete validated target Revision@R'
        v
PreparedRuntimeRevisionTransition
        |
        | final exact source freshness
        v
SealedRuntimeRevisionTransition
        | carries RevisionCommitDescriptor
        |
        | future WAL COMMIT/fsync
        v
publish() // infallible whole-root replacement
        |
        v
Revision@R' + PhysicalStore@R' + every materialization@R'
```

## 3. Implementation

Production files changed:

```text
Cargo.lock
crates/kernel-types/src/lib.rs
crates/kernel-query/src/lib.rs
crates/kernel-plan/Cargo.toml
crates/kernel-plan/src/lib.rs
```

### 3.1 MaterializationId

Added nominal `MaterializationId` to `kernel-types`. Maintained consumers now have explicit identity instead of being one anonymous root state.

### 3.2 RuntimeRevisionBundle now owns the actual Revision

`RuntimeRevisionBundle` now owns:

```text
kernel_revision::Revision
PhysicalStore
BTreeMap<SemanticId, LayoutBinding>
BTreeMap<MaterializationId, MaterializedRelPlanState>
```

`revision_id()` is derived from the owned `Revision`; it is no longer the semantic authority itself.

### 3.3 Checked bootstrap instead of trusted snapshots

The old constructor shape was replaced by `RuntimeRevisionBundle::build`.

The builder:

1. receives the validated authoritative `Revision`;
2. verifies the selected physical relation registry matches all schema relations exactly;
3. semantically compares every selected physical relation snapshot to the relation value represented by the logical revision;
4. binds the physical store to the revision only after the snapshot check succeeds;
5. constructs each registered `MaterializedRelPlanState` from the authoritative revision model and pinned semantic context;
6. discovers each materialization's `Scan` dependencies;
7. attaches authoritative storage handles for those dependencies;
8. binds every materialization to the same revision.

Therefore a caller can no longer inject an arbitrary maintained snapshot while claiming it represents revision `R`.

### 3.4 Target Revision is part of the transition request

`RevisionTransitionRequest` now contains:

```text
target_revision: &kernel_revision::Revision
mutations: &[RevisionRelationMutation]
registry: &SemanticRegistry
```

The source revision and semantic context come only from the live bundle.

Per-mutation physical layout selection was removed from the caller. The runtime owner selects the pinned layout from its own relation-layout registry.

### 3.5 Exact target logical-state validation

Before any candidate can be returned, `prepare_revision` clones the source authoritative state and semantically applies every declared `RelationDelta` using the pinned Γ semantics.

The resulting complete `DatabaseState` must equal `target_revision.state()` exactly. Otherwise preparation returns:

```text
LogicalRevisionMutationMismatch
```

This prevents the logical target revision and the runtime mutation descriptor from advancing independently.

The current Pass30 data-plane transition intentionally requires the target `SemanticContext` to equal the source context. Schema/Γ changes are rejected explicitly with:

```text
SemanticContextTransitionRequiresRebuild
```

They remain a separate OPEN migration/rebuild transaction problem rather than being silently interpreted as ordinary data mutation.

### 3.6 Materialization registry

All registered materializations are prepared in the same transaction candidate.

For each materialization:

1. its base `Scan` dependencies are collected;
2. only relevant storage-certified relation deltas are supplied to its recursive maintained state;
3. it is nevertheless advanced to the target revision even if the materialization output delta is empty;
4. its output delta is retained under its `MaterializationId`.

The published candidate therefore cannot contain one materialization at `R` and another at `R'`.

### 3.7 RevisionCommitDescriptor

A `RevisionCommitDescriptor` now crosses both prepared and sealed boundaries:

```text
source_revision
 target_revision
 semantic_revision
 BTreeMap<SemanticId, RelationDelta>
```

It is intentionally logical. It does **not** encode `StableRowHandle`, physical slot positions, or layout mutations as semantic authority.

This is the correct input shape for the later Agent-1 durability integration: future WAL serialization can be built around logical revision intent rather than current physical layout details.

### 3.8 Seal/publish law preserved

Pass29's final-freshness law remains intact.

`seal()`:

- compares the exact live bundle against the retained source snapshot;
- verifies source revision identity;
- on success retains exclusive mutable access to the live root.

`publish()`:

- is infallible;
- performs no semantic validation, row lookup, index planning, freshness check, or query maintenance;
- replaces the complete authoritative runtime bundle.

## 4. Hostile falsification

Pass30 added/strengthened hostile cases for the new authority boundary:

1. `bootstrap_rejects_logical_physical_divergence` — logical `Revision` and physical relation snapshot cannot disagree at construction;
2. `target_revision_state_must_match_logical_mutation_descriptor` — a target revision whose model is not exactly produced by the declared deltas is rejected before publication;
3. `materialization_registry_advances_all_registered_plans_atomically` — multiple materializations advance under one revision; an unaffected consumer may emit an empty delta but still becomes bound to `R'`;
4. `duplicate_materialization_id_is_rejected_at_bootstrap` — registry identity cannot alias two materializations;
5. all Pass29 stale/competing/batch rollback/handle-generation/abort-by-drop hostile tests continue to pass under the stronger owner.

`kernel-plan` now has **59** library tests.

## 5. Verification

Toolchain:

```text
rustc 1.98.1 (48a229cea 2026-09-01)
cargo 1.98.1 (797e8a9bc 2026-08-05)
rustfmt 1.9.0-stable (48a229ceae 2026-09-01)
clippy 0.1.98 (48a229ceae 2026-09-01)
```

Full gate:

1. `cargo fmt --all -- --check` — **PASS**;
2. `cargo check --workspace --all-targets` — **PASS**;
3. `cargo test --workspace --all-targets` — **PASS**;
4. `cargo clippy --workspace --all-targets -- -D warnings` — **PASS**;
5. `cargo test --workspace --all-targets --release` — **PASS**;
6. `cargo build --workspace --release` — **PASS**;
7. `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps` — **PASS**;
8. `RUSTFLAGS='-C overflow-checks=yes' cargo test --workspace --all-targets --release` — **PASS**.

Static workspace state:

- **241** declared workspace `#[test]` tests;
- **59** `kernel-plan` library tests;
- **19** workspace crates;
- **29,818** Rust LOC;
- **0** external Cargo source entries;
- **0** `unsafe` occurrences under `crates/`.

Raw verification logs are under `evidence/pass30/`.

## 6. Чеклист Pass30

### Закрыто именно в этом цикле

1. [x] `RuntimeRevisionBundle` теперь владеет actual validated `kernel_revision::Revision=(S,Γ,M)`, а не только `RevisionId`.
2. [x] Logical revision, selected physical store и maintained consumers теперь принадлежат одному publication root.
3. [x] Bootstrap больше не доверяет caller-supplied maintained snapshot: materializations строятся из authoritative revision.
4. [x] Bootstrap проверяет logical↔physical relation coherence до revision binding.
5. [x] Один runtime root теперь владеет registry нескольких maintained materializations с nominal `MaterializationId`.
6. [x] Все materializations переходят на target revision атомарно, включая consumers с пустым output delta.
7. [x] Target transaction input теперь является полным validated `Revision`, а не только target id.
8. [x] Declared relation deltas обязаны точно воспроизводить полный target logical state; mismatch отвергается.
9. [x] Caller больше не выбирает physical layout для semantic mutation: layout authority находится внутри runtime bundle.
10. [x] Через prepared/sealed boundary теперь проходит logical `RevisionCommitDescriptor`, пригодный как будущая durability authority boundary без physical handles.
11. [x] Duplicate materialization identity и logical/physical bootstrap divergence получили hostile falsifiers.

### Закрыто из OPEN-чеклиста Pass29: **4 / 11**

Закрыты:

1. [x] Authoritative full `Revision=(S,Γ,M)` ownership внутри runtime publication root.
2. [x] Complete logical relation-mutation descriptor доступен из prepared/sealed transition для будущего WAL boundary.
3. [x] Multiple maintained consumers получили единый materialization registry и all-or-nothing candidate.
4. [x] Bootstrap consistency больше не доверяется caller'у: logical/physical/maintained coherence конструктивно проверяется/перестраивается.

### Осталось из прошлого OPEN: **7 / 11**

1. [ ] Correctness-first full source/candidate cloning заменить на immutable/COW/version-root identity.
2. [ ] Cross-crate prepared-plan capability sealing.
3. [ ] Закрыть публичный minting `StorageCertifiedRelationDelta` как authority capability.
4. [ ] Реализовать Schema/Γ-changing revision transaction / rebuild-migration path.
5. [ ] Process/filesystem crash atomicity.
6. [ ] Production WAL/replay/checkpoint/segment recovery integration.
7. [ ] Конкретный concurrent reader publication primitive: immutable root/MVCC/RwLock/root swap.

### Новые / уточнённые OPEN Pass30

1. [ ] **Selected-layout multiplicity.** Authoritative bundle сейчас требует ровно один selected physical relation layout на каждую schema relation. Alternate reconstructible layouts/replicas должны получить отдельный derived registry, не смешанный с semantic authority.
2. [ ] **General logical revision changes.** `RevisionCommitDescriptor` Pass30 покрывает relation-data deltas при неизменном Γ. Lifecycle/field/schema/semantic migrations нуждаются в общем change descriptor либо отдельном строго типизированном transaction class.
3. [ ] **Durable encoding.** Descriptor теперь существует на правильной boundary, но stable binary codec/versioning/checksum принадлежат следующей WAL integration и ещё не production-realized.
4. [ ] **Bootstrap cost.** Coherence verification correctness-first и может быть линейно/квадратично дорогой для generic semantic collections; позже нужен certified version/digest/root fast path без ослабления exact check contract.
5. [ ] **Snapshot memory cost.** `PreparedRuntimeRevisionTransition` всё ещё удерживает полный source snapshot и полный candidate bundle; это корректно, но не production memory model.

### Отложенный project-wide OPEN

1. [ ] Agent-2 canonical semantic-index integration into production Join/Group/TopK.
2. [ ] Generic Text/F64/Bool/entity maintained indexes и generic Text/F64 TopK order-statistics.
3. [ ] Stateful Group/TopK typed-batch producer path и оставшиеся constant-factor gaps.
4. [ ] Remaining physical layout families + OrderedView/pagination.
5. [ ] Observation-driven transaction repair, distribution/replication и formal mechanization.

## 7. Recommended next mainline pass

Do not integrate Agent-2 yet.

The central owner is now semantically complete enough that the remaining pre-WAL work is narrower:

```text
Pass30 authoritative Revision root
        |
        v
seal remaining certificate/prepared capabilities
        |
        v
replace full-snapshot freshness with immutable/version-root identity
        |
        v
SealedRevisionCommit
        |
        v
Agent-1 logical WAL integration
```

The next mainline pass should therefore close **capability authority + versioned publication identity** rather than introduce another implementation branch. After that, Agent-1 WAL can be integrated directly into the already proven seam:

```text
prepare -> seal -> durable COMMIT/fsync -> infallible publish
```
