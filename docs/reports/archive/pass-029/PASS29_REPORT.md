# PASS29 REPORT — sealed multi-relation runtime revision publication

Status: **VERIFIED** on Rust **1.98.1**.

Pass29 begins only after reopening Pass28 under the supplied Rust toolchain, fixing the defects exposed by the real compiler/test gate, and obtaining a fully green Pass28 baseline.

## 1. Problem

Pass28 established a correctness-first prepared transition across one `PhysicalStore` mutation and one `MaterializedRelPlanState`, but its public shape still had four architectural defects for the next durability boundary:

1. one logical revision could mutate only one base relation;
2. physical and maintained state remained separate live owners supplied independently to commit;
3. freshness was checked inside the same operation that published the candidates, leaving no capability boundary at which a future WAL layer could durably commit **after** final freshness validation but **before** runtime publication;
4. callers supplied both source and target revision ids to the transition request, even though source identity should come from the live owner.

Agent-1's verified WAL research sharpened defect (3): after a durable COMMIT marker has reached stable storage, runtime publication must not discover a new `StalePreparedTransition`. Therefore the last fallible freshness check must precede the durable point and exclusive ownership must be retained until infallible publication.

## 2. Pass28 verification correction before Pass29

The supplied Rust 1.98.1 toolchain exposed real Pass28 issues. They were fixed before Pass29 was forked:

1. `rustfmt --check` required formatting changes — **CLOSED**;
2. Clippy rejected the many-argument transaction API — replaced with `StoragePlanTransitionRequest`, no lint suppression — **CLOSED**;
3. one hostile test incorrectly treated physical row order as semantic bag order — corrected to bag-equivalence — **CLOSED**;
4. stale-plan failure escaped as `Query(StalePreparedTransition)` rather than the central transaction error — normalized at the boundary — **CLOSED**.

The corrected Pass28 then passed debug/release tests, strict Clippy, release build, strict rustdoc, and overflow-checked release tests. Pass28 is therefore a real verified checkpoint rather than the earlier source-audited artifact.

## 3. Hypothesis

Make one runtime owner represent the coherent reader-visible revision and separate preparation from final freshness sealing:

```text
RuntimeRevisionBundle@R
    |
    | prepare several normalized relation mutations on candidates only
    v
PreparedRuntimeRevisionTransition
    |
    | final exact freshness check + acquire exclusive mutable borrow
    v
SealedRuntimeRevisionTransition<'live>
    |
    | future durable COMMIT may happen here
    | live bundle cannot be mutated through safe Rust while seal exists
    v
publish()  // infallible state replacement
    |
    v
RuntimeRevisionBundle@R'
```

A batch is one logical data-plane revision: at most one normalized mutation per semantic relation. If any relation, physical index update, maintained-tree propagation, or final freshness check fails, the live bundle remains unchanged.

## 4. Implementation

Production source changed in one crate/file:

```text
crates/kernel-plan/src/lib.rs
```

`kernel-query` required no Pass29 production change.

### 4.1 RuntimeRevisionBundle

Added `RuntimeRevisionBundle`, owning:

- the current `RevisionId`;
- `PhysicalStore`;
- `MaterializedRelPlanState`.

The bundle exposes read-only access to physical and maintained state. Semantic relation mutation is not exposed through independent mutable child references. Reconstructible `install_i64_index` is routed through the bundle and deliberately invalidates already prepared transitions through source-state freshness.

### 4.2 RevisionTransitionRequest

Added:

```text
RevisionRelationMutation {
    relation,
    layout,
    delta,
}

RevisionTransitionRequest {
    target_revision,
    mutations[],
    pinned context,
    semantic registry,
}
```

`source_revision` is no longer caller supplied. It is derived from the live `RuntimeRevisionBundle`.

A revision batch rejects duplicate semantic relation ids before any candidate mutation. Multiple edits to one relation must be normalized by the caller into one exact `RelationDelta`.

### 4.3 Multi-relation prepare

`RuntimeRevisionBundle::prepare_revision`:

1. rejects `target_revision == source_revision`;
2. rejects duplicate relation mutations;
3. verifies child revision bindings agree with the bundle revision;
4. clones the physical candidate once;
5. applies every base-relation mutation only to the candidate, collecting exact `StorageCertifiedRelationDelta`s keyed by relation;
6. prepares the recursive maintained tree against the full certified-delta map;
7. constructs one complete target bundle;
8. retains an exact source bundle snapshot for hostile freshness checking.

No live semantic state is mutated during preparation.

### 4.4 Prepare -> seal -> publish

`PreparedRuntimeRevisionTransition::seal` performs the last fallible freshness check against the **entire** live bundle, not only revision/epoch counters.

On success it returns `SealedRuntimeRevisionTransition<'_>`, which retains an exclusive mutable borrow of the live bundle. While that capability exists, safe Rust cannot independently mutate the same bundle before publication.

`SealedRuntimeRevisionTransition::publish` returns no `Result`. It replaces the complete bundle in one assignment and returns the already computed output delta. There is no semantic lookup, validation, index planning, freshness test, or fallible transaction operation after the seal boundary.

This is the required in-memory seam for future WAL integration:

```text
prepare -> seal -> WAL COMMIT/fsync -> publish
```

Pass29 does **not** itself implement WAL or claim process/filesystem crash durability.

### 4.5 Old single-relation joint API removed

The Pass28 public `PreparedStoragePlanTransition` / `prepare_storage_plan_transition` path was removed rather than retained as a second transaction mechanism. Its hostile invariants were migrated to the new bundle API.

The private unbound `PreparedPhysicalStoreTransition` remains only for legacy pre-revision-bound storage compatibility.

## 5. Hostile falsification

The migrated/new `kernel-plan` transaction tests cover:

1. prepare is invisible until sealed publication;
2. failed prepare leaves the live bundle unchanged;
3. a prepared transition cannot be sealed against a different bundle with the same numeric revision;
4. reconstructible index mutation after prepare makes the transition stale;
5. competing prepares from the same revision cannot both publish;
6. sequential revisions preserve stable slot generation semantics;
7. source==target revision is rejected without mutation;
8. dropping a sealed transition aborts without publication;
9. revision-bound legacy semantic mutation entrypoints remain blocked;
10. a two-relation revision publishes one coherent Join snapshot;
11. failure in the second relation of a batch leaves the first relation unpublished;
12. duplicate relation mutations are rejected before prepare.

The multi-relation Join test compares the maintained result against a fresh logical recomputation oracle after publishing both changed base relations.

## 6. Verification

Toolchain:

```text
rustc 1.98.1 (48a229cea 2026-09-01)
cargo 1.98.1 (797e8a9bc 2026-08-05)
rustfmt 1.9.0-stable (48a229ceae 2026-09-01)
clippy 0.1.98 (48a229ceae 2026-09-01)
```

All gates passed:

1. `cargo fmt --all -- --check` — **PASS**;
2. `cargo check --workspace --all-targets` — **PASS**;
3. `cargo test --workspace --all-targets` — **PASS**;
4. `cargo clippy --workspace --all-targets -- -D warnings` — **PASS**;
5. `cargo test --workspace --all-targets --release` — **PASS**;
6. `cargo build --workspace --release` — **PASS**;
7. `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps` — **PASS**;
8. `RUSTFLAGS='-C overflow-checks=yes' cargo test --workspace --all-targets --release` — **PASS**.

Static state:

- 55 `kernel-plan` library tests;
- 237 declared workspace `#[test]` tests;
- 19 workspace crates;
- 29,302 Rust LOC;
- 0 external Cargo source entries;
- 0 `unsafe` blocks.

Raw gate logs are under `evidence/pass29/`.

## 7. Чеклист Pass29

### Закрыто именно в этом цикле

1. [x] Pass28 впервые прогнан реальным Rust 1.98.1 gate и доведён до полностью зелёного VERIFIED baseline.
2. [x] Один runtime revision теперь может атомарно нести несколько base-relation mutations.
3. [x] `PhysicalStore` и текущий maintained plan больше не передаются как два независимых live publication owner; их объединяет `RuntimeRevisionBundle`.
4. [x] `source_revision` больше не выбирается caller'ом: он берётся из live bundle.
5. [x] Финальная freshness-проверка вынесена в `seal()` до будущей durable WAL boundary.
6. [x] После успешного `seal()` publication transaction API больше не имеет fallible semantic path: `publish()` infallible.
7. [x] Failure на N-й relation batch'а не публикует уже подготовленные relation 1..N-1.
8. [x] Competing prepared revisions от одного source snapshot не могут обе успешно publish.
9. [x] Старый Pass28 single-relation public transaction protocol удалён, чтобы не было двух authority boundaries.
10. [x] Специфическое Pass28 окно `storage replaced -> plan not yet replaced` устранено: runtime publication теперь заменяет один whole bundle.

### Закрыто из OPEN-чеклиста Pass28: **9 / 16**

Закрыты:

1. [x] compile Pass28 with Rust 1.98.1;
2. [x] `cargo fmt --all -- --check`;
3. [x] workspace debug/release tests;
4. [x] strict Clippy;
5. [x] release build;
6. [x] strict rustdoc;
7. [x] overflow-checked release tests;
8. [x] multi-relation / batch revision;
9. [x] recovery problem specific to a process dying between **two independent in-memory replacements** is structurally removed by one bundle replacement. This does **not** close general WAL/crash recovery.

### Осталось из прошлого OPEN: **7 / 16**

1. [ ] Integrate runtime publication ownership with authoritative `kernel_revision::Revision`, not only a `RevisionId`.
2. [ ] Replace correctness-first full source/candidate snapshots with immutable/COW/version-root identity without weakening stale falsifiers.
3. [ ] Cross-crate capability sealing for maintained-plan prepared transition internals.
4. [ ] Seal `StorageCertifiedRelationDelta` construction so only storage authority can mint certificates.
5. [ ] Support target-revision Schema/Γ changes under one prepared revision protocol.
6. [ ] Process/filesystem crash atomicity.
7. [ ] Production WAL/replay/checkpoint/segment recovery protocol integration.

### Новые / уточнённые проблемы Pass29

1. [ ] **Authoritative revision root.** `RuntimeRevisionBundle` currently owns `RevisionId + PhysicalStore + one maintained plan`, but not the validated logical `Revision = (S, Γ, M)`. Until this is fixed, semantic authority and runtime derivatives can still be advanced by separate higher-level code.
2. [ ] **Durable descriptor at the seal boundary.** Future WAL integration needs the complete logical revision mutation/revision descriptor available from `SealedRevisionCommit`; the current sealed runtime transition protects freshness but does not encode durable `(S, Γ, M/change)` authority.
3. [ ] **Multiple maintained consumers.** One maintained plan proves the mechanism, but a real revision owner needs a registry of materializations with explicit registry identity/freshness and all-or-nothing publication.
4. [ ] **Bootstrap consistency.** `RuntimeRevisionBundle::new` trusts that supplied physical and maintained snapshots represent the same logical revision; construction needs an authoritative builder/verification path.
5. [ ] **Reader publication primitive.** Safe-Rust exclusivity proves the writer-side seam, but the eventual reader-visible mechanism (`RwLock`, immutable root swap, MVCC root, etc.) is still intentionally unspecified.

### Отложенный project-wide OPEN, не закрываемый Pass29

1. [ ] Agent-2 semantic-index integration into production Join/Group/TopK.
2. [ ] Generic Text/F64 maintained TopK order-statistics and generic indexed Text/F64/Bool/entity Join/Group.
3. [ ] Stateful Group/TopK typed-batch producer path and remaining constant-factor gaps.
4. [ ] Remaining physical layouts, OrderedView/pagination.
5. [ ] Observation-based transaction repair, distribution/replication and formal mechanization.

## 8. Recommended next mainline pass

Do **not** integrate Agent-2 indexes yet and do not add more independent implementation agents.

The next mainline pass should promote `RuntimeRevisionBundle` from a runtime data-plane owner into a complete revision publication owner:

```text
Authoritative Revision@R
+ PhysicalStore@R
+ maintained materialization registry@R
            |
            | prepare logical revision + all runtime derivatives
            v
PreparedRevisionCommit
            |
            | final freshness / consistency seal
            v
SealedRevisionCommit
            |
            | future WAL durable point
            v
infallible one-root publication @R'
```

Only after that contract is verified should Agent-1 WAL be integrated. Agent-2 semantic indexes should follow durability/mainline transaction integration rather than being merged concurrently with it.
