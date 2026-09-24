# PASS32 REPORT — logical WAL tail + recovery integration

Status: **VERIFIED** on Rust **1.98.1**.

Pass32 continues directly from verified Pass31 and integrates the verified Agent-1 logical-WAL law into the mainline transaction boundary. Agent-2 semantic indexes remain deliberately deferred until the durability core is stable.

## 1. Problem

Pass31 closed the in-memory authority and publication boundary:

```text
RuntimeRevisionCell
  -> prepare detached candidate
  -> seal exact live root under sole writer guard
  -> infallible whole-root publish
```

The remaining durability problem was that a process/filesystem crash could still destroy the committed semantic revision, and the project had no production WAL codec/scanner/replay path. The Agent-1 prototype had already falsified the framing law independently, but it was not integrated into the actual `RevisionCommitDescriptor`, runtime seal, or recovery builder.

The critical integration constraints are:

1. durable authority must be the logical revision mutation, not physical row handles/indexes/materializations;
2. durable COMMIT must happen only after the last fallible runtime freshness check;
3. no semantic/freshness error may first appear after durable COMMIT;
4. a crash after PREPARE but before COMMIT must recover the previous durable revision;
5. a crash after durable COMMIT but before runtime publication/ACK must recover the committed revision;
6. recovery must create a fresh runtime lineage and reconstruct physical/maintained state rather than replay process-local identities as authority;
7. Pass26 historical OPEN items must remain in the project ledger even while the current mainline focuses on durability.

## 2. Hypothesis

Introduce a small std-only `kernel-durability` layer whose durable record is an exact, versioned logical relation-data mutation descriptor:

```text
runtime prepare
    -> DurableRevisionDescriptor
    -> WAL PREPARE + sync_data
    -> runtime seal / last freshness check
    -> WAL COMMIT(binding exact PREPARE) + sync_data
    -> infallible RuntimeRevisionCell root publication
```

Recovery starts from one **exact durable base `Revision=(S,Γ,M)`** supplied by the checkpoint layer and replays only the committed WAL tail:

```text
exact base Revision
   + validated committed logical WAL tail
   -> validated recovered Revision
   -> reconstruct PhysicalStore
   -> reconstruct maintained materializations
   -> fresh RuntimeRevisionBundle/root lineage
```

Pass32 intentionally supports relation-data transitions under an unchanged pinned semantic context. Durable schema/Γ/lifecycle migrations, checkpoint installation and segment management remain explicit later problems rather than being hidden inside an underspecified codec.

## 3. Implementation

Production source changed in:

```text
Cargo.toml
Cargo.lock
crates/kernel-durability/Cargo.toml
crates/kernel-durability/src/lib.rs
crates/kernel-plan/Cargo.toml
crates/kernel-plan/src/lib.rs
```

### 3.1 New `kernel-durability`

Added a zero-external-dependency durability crate.

Core durable objects:

```text
DurableRelationMutation
DurableRevisionDescriptor
DurablePrepareToken
DurableCommitReceipt
CommittedRevision
RecoveryScan
RevisionDurability
FileRevisionWal
SimulatedRevisionWal
```

The durable descriptor contains only:

```text
source RevisionId
target RevisionId
pinned SemanticRevision
exact logical relation row insertions/removals
```

It does **not** serialize:

- `StableRowHandle`;
- storage slot/generation;
- selected physical layout;
- physical indexes;
- materialized-plan internal state;
- process-local runtime root lineage/version.

Those remain reconstructible physical evidence.

### 3.2 Stable versioned logical mutation codec

Pass32 adds mutation codec version `1` and a deterministic codec covering every current `kernel_model::Value` variant:

```text
Unit
Bool
I64
F64Bits
Text
LiveEntityRef
HistoricalEntityId
Product
Option
Variant
Seq
Set
Bag
Map
```

Structural sizes and nesting are hard-limited during decode. Product ordering is canonical because semantic field IDs come from `BTreeMap` order. Relation mutations are encoded in their validated deterministic relation order.

The codec deliberately does not persist a host-derived relation result type. During replay, the type is re-derived from the exact pinned base semantic context.

### 3.3 WAL framing

Each frame has a fixed 36-byte versioned header containing:

```text
magic
format version
record kind
flags
payload length
LSN
revision id
payload CRC-32C
header CRC-32C
```

Current record kinds:

```text
PREPARE_REVISION
COMMIT_REVISION
```

COMMIT stores and validates:

```text
target revision
prepare LSN
prepare payload CRC-32C
```

so a COMMIT cannot silently bind a different PREPARE.

The scanner enforces monotone gap-free LSNs, frame/header/payload checksums, exact duplicate rules, source-revision chaining from the durable head, and one committed semantic transition per target revision.

### 3.4 File durability boundary

`FileRevisionWal` performs:

```text
PREPARE append -> File::sync_data()
COMMIT  append -> File::sync_data()
```

A file writer acquires an exclusive filesystem file lock. `create()` uses `create_new`, so accidentally calling creation on an existing WAL cannot truncate it.

If a frame write or durability barrier fails, the live WAL writer is poisoned and cannot continue as if ordering were known.

If `durably_commit` returns an I/O error, `kernel-plan` reports:

```text
CommitDurabilityUncertain
```

rather than claiming the revision aborted. The runtime candidate is not published and the caller must resolve the durable outcome by reopening/scanning the WAL.

Pass32 does not claim directory-entry durability for first segment creation; parent-directory fsync and manifest/segment lifecycle remain OPEN.

### 3.5 Recovery scanner

`scan_wal(bytes, base_revision)` returns a validated `RecoveryScan` containing only the durable committed prefix plus tail status.

Tail policy:

- incomplete/torn final frame: safe truncated tail;
- short non-frame garbage tail: ignored after the last validated frame and removable on recovery;
- enough non-frame bytes to hide a complete frame: corruption, not silently ignored;
- corrupt header/payload checksum: corruption;
- non-monotone/gapped LSN: protocol error;
- conflicting duplicate PREPARE/COMMIT: protocol error;
- exact duplicate PREPARE/COMMIT: idempotently accepted.

`FileRevisionWal::open_recovered` scans first, truncates only a scanner-certified safe tail to `last_good_offset`, calls `sync_all`, seeks to the end and resumes at the scanner-derived next LSN.

### 3.6 Runtime integration

`RevisionCommitDescriptor::durable_descriptor()` converts the already validated mainline logical mutation into `DurableRevisionDescriptor`.

`RuntimeRevisionCell::commit_revision_durable` now executes the production order:

```text
1. prepare detached runtime candidate
2. durable PREPARE + barrier
3. seal against exact live root / acquire sole writer publication guard
4. durable COMMIT + barrier
5. infallible whole-root publish
```

This ordering intentionally allows stale detection after a durable PREPARE but **before** durable COMMIT. Such an orphan PREPARE is ignored by recovery.

After seal succeeds, no runtime semantic/freshness work remains. Durable COMMIT therefore precedes only the infallible root replacement.

### 3.7 Logical replay + runtime rebuild

Added:

```text
replay_durable_revisions(...)
recover_runtime_bundle(...)
```

Replay requires the exact base revision ID and exact pinned semantic context. For every committed descriptor it:

1. verifies descriptor source equals the current recovered head;
2. verifies the descriptor semantic revision equals the current exact context;
3. re-derives each relation type from the pinned semantic context;
4. applies the logical relation delta;
5. rebuilds a validated `kernel_revision::Revision` for the target ID.

The runtime recovery builder then reconstructs physical relations and registered maintained materializations from the recovered semantic authority.

The first integration test exposed a real bug: recovery initially paired `NativeRelation::RowStore` with the logical pseudo-layout `LayoutFamily::LogicalModelRows`. That was an invalid physical family binding. Pass32 fixes this with explicit reconstructible:

```text
LayoutBinding::RECOVERY_ROW_STORE
```

Recovery therefore does not pretend that logical model storage is itself a native physical layout.

## 4. Hostile falsification

`kernel-durability` now has **13** dedicated tests, including:

1. CRC-32C standard known vector;
2. stable codec round-trip across every current `Value` shape;
3. every byte-prefix cut of multiple committed revisions exposes only an actually committed prefix;
4. durable PREPARE without COMMIT remains invisible;
5. every single-bit corruption of a committed stream is rejected;
6. COMMIT whose source does not equal the durable head is rejected;
7. exact duplicate PREPARE + exact duplicate COMMIT are idempotent;
8. conflicting duplicate PREPARE is rejected;
9. conflicting duplicate COMMIT is rejected;
10. short garbage tail is distinguished from enough garbage to hide a full frame;
11. valid-checksum but non-monotone LSN is rejected;
12. `create()` cannot truncate an existing WAL;
13. file recovery truncates only a safe torn tail and resumes the correct LSN.

`kernel-plan` adds **4** integrated durability/recovery hostile tests:

1. durable COMMIT linearizes before runtime publication and logical WAL recovery reproduces the semantic result;
2. multiple nonconsecutive `RevisionId`s replay correctly and replay from the same exact base is idempotent;
3. COMMIT durability failure after seal never publishes the runtime candidate and is reported as uncertain;
4. a reconstructible physical root update after durable PREPARE makes seal stale, leaving only an uncommitted WAL PREPARE.

Recovery tests additionally verify that reconstructed physical representation may differ from the pre-crash one while authoritative `Revision` and maintained result remain equivalent.

## 5. Verification

Toolchain:

```text
rustc 1.98.1 (48a229cea 2026-09-01)
cargo 1.98.1 (797e8a9bc 2026-08-05)
rustfmt 1.9.0-stable (48a229ceae 2026-09-01)
clippy 0.1.98 (48a229ceae 2026-09-01)
```

Final gate after the last source change:

1. `cargo fmt --all -- --check` — **PASS**;
2. `cargo check --workspace --all-targets` — **PASS**;
3. `cargo test --workspace --all-targets` — **PASS**;
4. `cargo clippy --workspace --all-targets -- -D warnings` — **PASS**;
5. `cargo test --workspace --all-targets --release` — **PASS**;
6. `cargo build --workspace --release` — **PASS**;
7. `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps` — **PASS**;
8. `RUSTFLAGS='-C overflow-checks=yes' cargo test --workspace --all-targets --release` — **PASS**.

Earlier combined/overflow release attempts that hit the external execution timeout while compiling are retained as diagnostics and are not counted as failed tests; every corresponding gate was rerun separately to completion after the final code change.

Static workspace state:

- **260** declared workspace `#[test]` tests;
- **65** `kernel-plan` library tests;
- **13** `kernel-durability` tests;
- **20** workspace crates;
- **31,959** Rust LOC;
- **0** external Cargo source entries;
- **0** `unsafe` occurrences under `crates/`.

Raw logs: `evidence/pass32/`.

## 6. Чеклист Pass32

### Закрыто именно в этом цикле

1. [x] Agent-1 logical-authoritative WAL law интегрирован в mainline `RuntimeRevisionCell` transaction boundary.
2. [x] Добавлен production `kernel-durability` без внешних Cargo-зависимостей.
3. [x] Добавлен stable/versioned logical relation-mutation codec для всех текущих `Value` shapes.
4. [x] Добавлены PREPARE/COMMIT WAL frames с monotone LSN, header CRC-32C и payload CRC-32C.
5. [x] COMMIT криптографически не заявляется, но точно связывает target revision с конкретным PREPARE LSN + payload checksum.
6. [x] Durable ordering mainline теперь: runtime prepare -> durable PREPARE -> seal -> durable COMMIT -> infallible publish.
7. [x] Stale transition после durable PREPARE не публикуется и оставляет только recovery-ignored uncommitted PREPARE.
8. [x] COMMIT I/O failure после seal не публикует runtime candidate и явно маркируется `CommitDurabilityUncertain`.
9. [x] Recovery scanner находит только validated committed prefix и отвергает checksum/protocol corruption.
10. [x] Exact duplicate PREPARE/COMMIT являются idempotent; conflicting duplicates отвергаются.
11. [x] Safe torn/short-garbage tail можно отрезать; полноценный скрытый garbage frame не принимается молча.
12. [x] `FileRevisionWal` использует exclusive file lock и не может случайно truncate существующий WAL через `create()`.
13. [x] Logical WAL replay работает от exact base `Revision`, а не от process-local root identity.
14. [x] Recovery reconstructs `PhysicalStore` и maintained materializations; row handles/indexes не становятся durable authority.
15. [x] Recovery-layout bug (`LogicalModelRows` vs native RowStore) найден hostile integration test'ом и закрыт через explicit `RECOVERY_ROW_STORE`.
16. [x] Stable process-local `root_id/version` не сериализуются в WAL и после recovery создаётся новый runtime lineage.

### Закрыто из OPEN-чеклиста Pass31: **1 / 8 полностью + 1 крупный составной пункт существенно сужен**

Полностью закрыт:

1. [x] Stable durable encoding/versioning/checksum для relation-data `RevisionCommitDescriptor` — реализован production codec + frame checksums.

Составной durability OPEN из Pass31 декомпозирован:

2. [x] Production logical WAL tail, PREPARE/COMMIT barriers, scanner, exact-base replay и runtime rebuild — **CLOSED в Pass32**.

Но исходный пункт `WAL/recovery/checkpoint/segment integration` **ещё не закрыт целиком**, потому что durable base checkpoint creation/installation, segment rotation/truncation manifest и real filesystem crash validation остаются OPEN.

### Осталось из прошлого OPEN Pass31

1. [ ] Candidate construction перевести с correctness-first deep clones на COW/persistent immutable subroots.
2. [ ] Schema/Γ-changing revision transaction / rebuild-migration path.
3. [ ] Durable checkpoint creation/installation + parent-directory fsync/rename protocol.
4. [ ] WAL segment rotation/truncation/manifest lifecycle.
5. [ ] Real process/filesystem/power-loss crash falsification on target filesystems.
6. [ ] Selected-layout multiplicity: alternate reconstructible layouts/replicas/index families вынести в отдельный derived registry.
7. [ ] Bootstrap coherence check ускорить certified version/digest/root fast path с exact fallback.
8. [ ] Общий logical change descriptor для lifecycle/field/schema/semantic changes либо отдельные typed durable migration transactions.
9. [ ] RwLock poisoning/restart policy состыковать с recovery owner.
10. [ ] Legacy unbound `PhysicalStore` clone-heavy prepared path остаётся performance cleanup.

### Исторический OPEN backlog из Pass26 — production status

Эти пункты продолжают жить в ledger:

1. [ ] Generic Text/F64 maintained TopK order-statistics.
2. [ ] I64 TopK constant-factor gap.
3. [ ] Group/TopK as typed-batch producers.
4. [ ] Maintained I64 Group constant-factor gap.
5. [ ] Indexed generic/Text Group.
6. [ ] Persisted Text/F64/Bool/entity indexes + planner.
7. [ ] Nested/multiway/mixed-key joins.
8. [ ] Remaining physical layouts + OrderedView/pagination.
9. [ ] WAL/recovery/durable materializations/crash tests — **PARTIALLY CLOSED:** Agent-1 law + production WAL tail/replay/runtime materialization rebuild are now VERIFIED; durable checkpoints/segments + real crash tests remain OPEN, so the historical composite item stays OPEN.
10. [ ] Transaction repair runtime, distribution, formal mechanization — local transaction/durable-recovery core advanced substantially; repair/distribution/formal parts remain OPEN.
11. [ ] Semantic indexing/canonical-key strategy for generic maintained Join — **Agent-2 canonical-key/index feasibility R&D VERIFIED; production Join integration remains OPEN**.

Pass26's additional `storage→maintained-plan leaf contract` remains **CLOSED since Pass27**.

### Новые / уточнённые OPEN Pass32

1. [ ] **Durable base checkpoint authority.** Recovery currently requires an exact trusted base `Revision`; Pass32 does not yet serialize/install/checkpoint that object and its full schema/Γ dependencies.
2. [ ] **Segment creation directory durability.** WAL file frames are fsynced, but initial directory entry/manifest durability across filesystems requires explicit parent-directory fsync/rename policy.
3. [ ] **Revision DAG durability.** Current production WAL is a linear runtime-head tail with arbitrary/nonconsecutive revision IDs; durable branch/merge parent sets require a DAG record contract.
4. [ ] **ACK/idempotency ambiguity.** Crash after durable COMMIT but before client acknowledgement needs a client transaction/idempotency identifier above `RevisionId`.
5. [ ] **Schema/Γ/lifecycle codec.** Pass32 durable payload covers relation-data mutation under unchanged pinned semantic context only.
6. [ ] **Materialization configuration authority.** Recovery receives materialization specs from the caller; their durable/configuration source must be defined before standalone restart is complete.
7. [ ] **Recovered performance state.** Recovery intentionally rebuilds generic row-store physical state; performance indexes/layout replicas need reconstructible rebuild scheduling.
8. [ ] **CRC threat model.** CRC-32C detects accidental corruption; malicious tamper/authentication requires a separate digest/MAC/signature policy if that threat model is in scope.
9. [ ] **Group commit / async durability / replication.** Current barriers are correctness-first one-revision synchronous fsyncs.
10. [ ] **Recovery ownership after `CommitDurabilityUncertain`.** API correctly refuses to infer abort/commit, but higher-level engine restart/reopen workflow is not yet packaged as one supervisor operation.

## 7. Result / next mainline pass

Pass32 closes the first production durability slice: a committed logical relation-data revision can now survive runtime loss through a validated WAL tail and rebuild a fresh authoritative runtime root.

The next mainline dependency should remain durability, not semantic-index integration:

```text
Pass32 VERIFIED
  logical WAL tail
  + exact-base replay
  + runtime rebuild
       |
       v
Pass33: durable checkpoint + segment lifecycle + restart owner
  checkpoint install / manifest / parent-dir fsync
  segment rotation / truncation
  recovery owner for uncertain commit
       |
       v
real crash/fault falsification
       |
       v
schema/Γ durable migration class
       |
       v
Agent-2 semantic-index production integration
```

The Pass26 performance/features backlog remains active and must not be dropped while durability is completed.
