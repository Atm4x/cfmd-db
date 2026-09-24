# PASS31 REPORT — versioned immutable runtime root + capability sealing

Status: **VERIFIED** on Rust **1.98.1**.

Pass31 continues directly from verified Pass30. It deliberately does **not** integrate Agent-1 WAL or Agent-2 semantic indexes. The purpose of this pass is to close the remaining in-memory publication/capability boundary before durability is attached.

## 1. Problem

Pass30 made `RuntimeRevisionBundle` authoritative for:

```text
Revision=(S, Γ, M)
+ selected PhysicalStore
+ MaterializationRegistry
```

and proved a `prepare -> seal -> infallible publish` transaction law. However, five gaps remained at the boundary:

1. prepared freshness retained a full source-bundle snapshot and compared whole state;
2. there was no concrete reader-visible publication primitive proving that a reader sees one coherent old or new root rather than mixed fields;
3. cross-crate maintained-plan preparation still exposed a prepared capability shape outside the runtime owner;
4. `StorageCertifiedRelationDelta` was named and shaped like an authority token even though it was only row-identity evidence;
5. a future WAL COMMIT needs a final exclusive publication capability whose freshness cannot change between `seal` and `publish`.

The old Pass26 project-wide performance/features backlog must also remain explicit. Agent research may close feasibility/R&D questions, but an item is not removed from the production OPEN ledger until mainline integration closes it.

## 2. Hypothesis

Use a nominal process-local runtime root lineage plus monotonically increasing root version for freshness, and publish the entire coherent bundle by swapping one immutable `Arc` root under a single writer lock:

```text
RuntimeRevisionCell
    |
    +-- Arc<RuntimeRevisionBundle root_id=L, version=V>
    |
    +-- snapshot() -> Arc clone for readers

prepare @ (L,V)
    -> PreparedRuntimeRevisionTransition {
           source_identity=(L,V),
           candidate=(L,V+1),
           logical RevisionCommitDescriptor
       }

seal(cell)
    -> acquire sole writer guard
    -> prove live identity == (L,V)
    -> SealedRuntimeRevisionTransition

       [future WAL COMMIT/fsync here]

publish()
    -> infallible single Arc-root replacement
```

A storage-to-query handoff should be named as non-authoritative evidence, not a semantic certificate. Public construction is safe only if it cannot mutate/publish a revision-bound root and every detached maintained candidate validates supplied handles against its current leaf snapshot.

## 3. Implementation

Production source changed in:

```text
crates/kernel-plan/src/lib.rs
crates/kernel-query/src/lib.rs
crates/kernel-plan/examples/storage_plan_bridge_bench.rs
```

### 3.1 Runtime root lineage and version

Added:

```text
RuntimeRootIdentity { root_id, version }
RuntimeRootVersion
```

`root_id` is allocated from a process-local monotone `AtomicU64`; it is deliberately **runtime freshness identity only**, not durable semantic identity. A successful semantic publication increments `version`. Reconstructible physical-index publication also increments the same root version while keeping the semantic `Revision` unchanged.

`PreparedRuntimeRevisionTransition` no longer stores `Box<RuntimeRevisionBundle>` as its source snapshot. It stores only the nominal source root identity plus the detached candidate.

This removes full-source-bundle equality from the runtime stale check and prevents a prepared transition from being presented to an independently constructed runtime that happens to contain byte/equality-equivalent semantic/physical state.

### 3.2 Concrete reader publication primitive

Added:

```text
RuntimeRevisionCell {
    RwLock<Arc<RuntimeRevisionBundle>>
}

RuntimeRevisionSnapshot {
    Arc<RuntimeRevisionBundle>
}
```

Readers clone the current `Arc` while holding a read lock only for the clone operation. Publication swaps one whole `Arc` under the write lock. Existing readers therefore retain the complete old root; subsequent readers receive the complete new root.

The authoritative runtime mutation entrypoint is now `RuntimeRevisionCell::prepare_revision`. The underlying bundle preparation method is private.

### 3.3 Seal is the exclusive pre-durable capability

`PreparedRuntimeRevisionTransition::seal(&RuntimeRevisionCell)`:

1. acquires the sole writer guard;
2. compares live `RuntimeRootIdentity` with the prepared source identity;
3. verifies source revision identity;
4. returns `StalePreparedTransition` before any future durability point if freshness fails.

`SealedRuntimeRevisionTransition` owns that write guard until `publish` or drop. While it exists, neither semantic publication nor reconstructible root mutation through the cell can advance the root.

`publish()` performs only:

```text
*live_root = Arc::new(candidate)
```

plus returning already-prepared materialization output deltas. It performs no row lookup, semantic validation, freshness check, query maintenance, or physical planning.

This preserves the future WAL seam:

```text
prepare -> seal -> durable COMMIT/fsync -> infallible publish
```

### 3.4 Cross-crate prepared-plan capability sealing

The query-layer `PreparedMaterializedRelPlanTransition` is no longer public.

The runtime boundary now calls a detached candidate API:

```text
candidate_from_storage_resolved_deltas_for_revision(...)
    -> (MaterializedRelPlanState, RelationDelta)
```

This can construct a candidate, but cannot publish it into `RuntimeRevisionCell` and cannot acquire the runtime publication capability.

### 3.5 `StorageCertifiedRelationDelta` removed as an authority concept

The type was renamed throughout production code to:

```text
StorageResolvedRelationDelta
```

and APIs were renamed from `*_certified_*` to `*_resolved_*`.

The new contract is explicit:

- it is row-identity evidence carrying exact row payload plus stable storage handles;
- it is **not** a semantic certificate;
- it is **not** a revision publication capability;
- callers may construct evidence with `from_parts`;
- maintained Scan leaves validate supplied handles against their current handle snapshot;
- a revision-bound maintained state rejects the old direct mutation path;
- only `RuntimeRevisionCell` owns authoritative revision publication.

Therefore public evidence construction is no longer an authority escape by construction; hiding the constructor is not required for semantic safety.

### 3.6 Reconstructible physical index publication

`RuntimeRevisionCell::install_i64_index` now:

1. takes the writer lock;
2. clones/updates the physical candidate;
3. increments root version without changing semantic `Revision`;
4. swaps the complete runtime root.

Any semantic transition prepared before that reconstructible mutation becomes stale. Old readers retain the old coherent root.

## 4. Hostile falsification

Pass31 adds/strengthens the following cases:

1. `reader_snapshot_remains_on_old_root_after_atomic_publication` — a reader acquired before publication retains old logical/physical/maintained state while a later reader sees the complete new root;
2. `prepared_transition_cannot_cross_identical_runtime_root_lineages` — independently built but semantically identical roots do not accept one another's prepared transition;
3. `physical_index_change_after_prepare_makes_transition_stale` — same semantic revision but newer reconstructible root version invalidates prepared work;
4. all previous competing-prepare, abort-after-seal, multi-relation rollback, stable-handle-generation, logical-target and multi-materialization tests remain green;
5. existing forged resolved-evidence hostile tests continue to validate handles before detached query maintenance and revision-bound legacy mutation remains rejected.

`kernel-plan` now has **61** library tests.

## 5. Verification

Toolchain:

```text
rustc 1.98.1 (48a229cea 2026-09-01)
cargo 1.98.1 (797e8a9bc 2026-08-05)
rustfmt 1.9.0-stable (48a229ceae 2026-09-01)
clippy 0.1.98 (48a229ceae 2026-09-01)
```

Final full gate:

1. `cargo fmt --all -- --check` — **PASS**;
2. `cargo check --workspace --all-targets` — **PASS**;
3. `cargo test --workspace --all-targets` — **PASS**;
4. `cargo clippy --workspace --all-targets -- -D warnings` — **PASS**;
5. `cargo test --workspace --all-targets --release` — **PASS**;
6. `cargo build --workspace --release` — **PASS**;
7. `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps` — **PASS**;
8. `RUSTFLAGS='-C overflow-checks=yes' cargo test --workspace --all-targets --release` — **PASS**.

Two combined release commands hit the external command time limit while compiling; they are preserved as interrupted diagnostic logs. The corresponding release and overflow-checked test commands were rerun separately to completion and passed. They are not counted as failed verification.

Static workspace state:

- **243** declared workspace `#[test]` tests;
- **61** `kernel-plan` library tests;
- **19** workspace crates;
- **30,053** Rust LOC;
- **0** external Cargo source entries;
- **0** `unsafe` occurrences under `crates/`.

Raw verification logs are under `evidence/pass31/`.

## 6. Чеклист Pass31

### Закрыто именно в этом цикле

1. [x] Reader-visible publication теперь имеет конкретный one-root primitive: `RwLock<Arc<RuntimeRevisionBundle>>`.
2. [x] Reader, уже получивший snapshot, конструктивно остаётся на старом coherent root после публикации новой revision.
3. [x] Runtime prepared freshness больше не требует полного source-bundle clone/equality; используется nominal `(root lineage, root version)` identity.
4. [x] Независимо построенные одинаковые runtime roots имеют разные lineages и не могут взаимозаменять prepared transitions.
5. [x] Reconstructible physical-index mutation публикуется whole-root swap и инвалидирует ранее prepared semantic transition через root version.
6. [x] `seal()` является exclusive pre-durable capability: после успешного seal live root не может стать stale до publish/drop guard.
7. [x] `publish()` после seal остаётся infallible и сводится к единой reader-visible root replacement.
8. [x] Cross-crate `PreparedMaterializedRelPlanTransition` больше не является public capability.
9. [x] `StorageCertifiedRelationDelta` удалён как ложный authority concept и заменён на явно non-authoritative `StorageResolvedRelationDelta` evidence.
10. [x] Public row-handle evidence больше не трактуется как semantic authority; authoritative publication доступна только через runtime root transaction.
11. [x] Старый Pass27 joint-publication OPEN теперь закрыт не только in-memory candidate law, но и конкретной reader-visible atomic root publication.
12. [x] Старый Pass27 certificate-authority OPEN закрыт архитектурно: сертификата/authority token больше нет; существует только проверяемое detached evidence.

### Закрыто из OPEN-чеклиста Pass30: **3 / 7 полностью + 1 частично**

Закрыты полностью:

1. [x] Cross-crate prepared-plan capability sealing.
2. [x] Public `StorageCertifiedRelationDelta` minting как authority capability — концепт удалён и заменён non-authoritative resolved evidence.
3. [x] Конкретный concurrent reader publication primitive / whole-root swap.

Закрыта часть комбинированного memory OPEN:

4. [x] Полный **source runtime snapshot** больше не удерживается для freshness; заменён root lineage/version identity.

Осталось из этого комбинированного пункта: candidate state всё ещё строится correctness-first через глубокие clones, а legacy unbound `PhysicalStore` prepared path сохраняет локальный source clone. Поэтому весь COW/memory пункт ещё не считается production-closed.

### Осталось из прошлого OPEN Pass30

1. [ ] Candidate construction перевести с correctness-first deep clones на COW/persistent immutable subroots без ослабления exact candidate law.
2. [ ] Schema/Γ-changing revision transaction / rebuild-migration path.
3. [ ] Process/filesystem crash atomicity.
4. [ ] Production WAL/replay/checkpoint/segment recovery integration.
5. [ ] Durable stable encoding/versioning/checksum для logical `RevisionCommitDescriptor`.
6. [ ] Selected-layout multiplicity: alternate reconstructible layouts/replicas/index families вынести в отдельный derived registry.
7. [ ] Bootstrap coherence check ускорить certified version/digest/root fast path без потери exact fallback.
8. [ ] Общий logical change descriptor для lifecycle/field/schema/semantic changes либо отдельные typed migration transactions.

### Исторический OPEN backlog из Pass26 — production status

Эти пункты **не удаляются** из ledger только потому, что поздние pass'ы сфокусировались на transaction core:

1. [ ] Generic Text/F64 maintained TopK order-statistics.
2. [ ] I64 TopK constant-factor gap.
3. [ ] Group/TopK as typed-batch producers.
4. [ ] Maintained I64 Group constant-factor gap.
5. [ ] Indexed generic/Text Group.
6. [ ] Persisted Text/F64/Bool/entity indexes + planner.
7. [ ] Nested/multiway/mixed-key joins.
8. [ ] Remaining physical layouts + OrderedView/pagination.
9. [ ] WAL/recovery/durable materializations/crash tests — **Agent-1 protocol R&D VERIFIED; production integration remains OPEN**.
10. [ ] Transaction repair runtime, distribution, formal mechanization — transaction publication core advanced substantially, but repair/distribution/formal parts remain OPEN.
11. [ ] Semantic indexing/canonical-key strategy for generic maintained Join — **Agent-2 canonical-key/index feasibility R&D VERIFIED; production Join integration remains OPEN**.

Pass26's additional `storage→maintained-plan leaf contract` problem is **CLOSED since Pass27** and in Pass31 its cross-layer object is clarified as non-authoritative resolved row-identity evidence.

### Новые / уточнённые OPEN Pass31

1. [ ] **Runtime root identity is not durable identity.** `root_id/version` are process-local freshness coordinates and must never be serialized or substituted for `RevisionId`/logical WAL authority.
2. [ ] **RwLock poisoning policy.** A panic while a writer guard is held yields `RuntimePublicationPoisoned`; durability/recovery integration must define restart/recovery policy rather than silently continuing.
3. [ ] **Publication primitive performance.** `RwLock<Arc<_>>` closes correctness, but a later lock-free/epoch/root-swap implementation may be justified by benchmark evidence; this is optimization, not an unresolved atomicity law.
4. [ ] **Detached resolved evidence validation law must remain explicit.** `StorageResolvedRelationDelta` is constructible by design; any future consumer must validate it and must not promote it into authority.
5. [ ] **Legacy unbound PhysicalStore path still clones source/candidate locally.** It is outside revision-bound authority but remains a performance cleanup target.

## 7. Result / next mainline pass

Pass31 closes the remaining **in-memory authority/publication** prerequisite that was blocking clean WAL integration.

The mainline dependency is now:

```text
Pass31 VERIFIED
  authoritative Revision root
  + versioned coherent reader snapshots
  + capability-sealed prepare/seal/publish
          |
          v
Pass32: integrate Agent-1 logical-authoritative WAL
  sealed descriptor -> PREPARE/COMMIT/fsync -> publish
  recovery -> rebuild authoritative runtime root
          |
          v
checkpoint/segment/crash falsification
          |
          v
Agent-2 semantic-index production integration
```

Do not remove the inherited Pass26 performance/features backlog. It remains the project-wide production ledger and should be resumed after the transaction/durability core is complete.
