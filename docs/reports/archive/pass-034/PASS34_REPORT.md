# PASS34 REPORT — durable checkpoint generations + manifest publication + restart owner

Status: **VERIFIED** on Rust **1.98.1**.

Pass34 resumes the durability line after Pass33 repaired the hostile storage→maintained leaf defects. The goal is to remove the last externally supplied “trusted base Revision” from normal restart flow and make one durable generation self-describing:

```text
checkpoint-N (exact Revision=(S,Γ,M))
+ wal-N (logical committed tail)
+ manifest-N (published generation authority)
```

A new `DurableRuntime` owns the runtime publication cell and its matching `DurableRevisionStore` together, so the mainline API cannot accidentally commit one runtime lineage through an unrelated durable WAL head.

## 1. Problem

Pass32/33 recovery still required an exact base `Revision` to be supplied from outside the durability subsystem. The WAL itself was crash-safe as a logical tail, but there was no production owner for:

1. exact durable base checkpoint encoding;
2. checkpoint/WAL generation publication;
3. parent-directory durability ordering;
4. segment rotation/compaction;
5. reopening checkpoint + WAL tail as one restart operation;
6. binding one live runtime to exactly one durable head.

That meant WAL replay was integrated, but a complete restart authority chain was not.

## 2. Hypothesis

Use immutable, monotonically numbered durable generations rather than rewriting/truncating the active WAL in place:

```text
generation N
  checkpoint-N.cfcp   exact validated Revision
  wal-N.cfmw          logical tail after checkpoint revision
  manifest-N.cfmf     publication record
```

Checkpoint rotation uses this ordering:

```text
write checkpoint-N+1
fsync checkpoint
create empty wal-N+1
fsync WAL
fsync directory              # prerequisite names durable
write pending manifest
fsync pending manifest
atomic rename -> manifest-N+1
fsync directory              # publication durable
```

Only the highest published `manifest-*.cfmf` is authority. Pending manifests and checkpoint/WAL files without a published manifest are orphans, not semantic authority.

The live runtime is wrapped as:

```text
DurableRuntime {
    RuntimeRevisionCell,
    Mutex<DurableRevisionStore>,
}
```

so commit, checkpoint and restart all share one durable-head contract.

## 3. Implementation

Production/config source changed in:

```text
crates/kernel-schema/src/lib.rs
crates/kernel-durability/Cargo.toml
crates/kernel-durability/src/lib.rs
crates/kernel-durability/src/checkpoint.rs      NEW
crates/kernel-durability/src/store.rs           NEW
crates/kernel-plan/src/lib.rs
Cargo.lock
```

No external Cargo dependency was added.

### 3.1 Exact Revision checkpoint codec

`kernel-durability` now has a versioned checkpoint codec for the complete current `Revision=(S,Γ,M)` authority.

It persists and reconstructs:

- `RevisionId`;
- full `Schema`, including symbols, type definitions, capabilities, fields, relations, structural equivalences and inclusions;
- full `SemanticEnvironment`, including pinned module digests;
- lifecycle entities/roots/keeps-alive graph;
- model carriers, fields and relations;
- every current `Value` shape through the existing stable Value codec.

Decode does not manufacture a trusted object directly. It reconstructs `SemanticContext + DatabaseState` and calls `Revision::build(...)` against the caller's `SemanticRegistry`, so schema/context/typed-model validation runs again during recovery.

`kernel-schema` gained read-only deterministic iterators needed for canonical serialization:

```text
symbols()
type_definitions()
capabilities()
inclusions()
```

They expose no mutation authority.

### 3.2 Checkpoint file framing

A checkpoint file has its own magic/version/header, exact payload length, payload CRC-32C and header CRC-32C. The manifest additionally pins the whole checkpoint-file CRC.

A published checkpoint with any of these mismatches is corruption; recovery does not silently fall back to an older generation once the newer manifest itself is published.

### 3.3 Immutable generation manifest

Manifest files are immutable and generation-numbered. There is no overwrite of the previous authoritative manifest.

Publication is two-stage:

```text
pending-manifest-<generation>.tmp
  -- fsync -->
rename
  --> manifest-<generation>.cfmf
  -- directory fsync --> durable publication
```

A crash before rename may leave pending/orphan artifacts, but they are ignored. A crash after the final manifest is visible can only reference checkpoint/WAL names that were already directory-fsynced before manifest publication.

### 3.4 WAL generation rotation

`DurableRevisionStore::rotate_checkpoint(revision)` is allowed only when:

```text
revision.id() == durable_store.durable_head()
```

It writes the current durable head as the next exact checkpoint, creates a fresh empty WAL segment based at that revision, then publishes the new manifest.

The next generation number is chosen above **all** existing checkpoint/WAL/manifest/pending artifacts, not simply `active + 1`; therefore a crash that leaves an unpublished orphan generation cannot block the next valid rotation.

### 3.5 Compaction

After a successful generation switch, `compact_obsolete_generations()` may delete non-active generation artifacts and stale pending manifests, then fsync the directory.

Compaction does not rewrite the active checkpoint or active WAL. Therefore crash safety does not depend on in-place truncation of the current authority files.

### 3.6 Published-generation recovery validation

`DurableRevisionStore::open` now validates:

```text
highest manifest generation
-> manifest CRC/version
-> exact referenced checkpoint exists
-> whole checkpoint checksum matches manifest
-> checkpoint header/payload checksums
-> decoded Revision validates against SemanticRegistry
-> manifest base revision == checkpoint RevisionId
-> referenced WAL file exists
-> WAL committed-prefix scan
```

A missing WAL referenced by a published manifest is corruption; `open_recovered` is not allowed to silently create an empty replacement segment.

### 3.7 Unified DurableRuntime owner

New public production owner:

```text
DurableRuntime::create(root, directory)
DurableRuntime::open(directory, materialization_specs, registry)
DurableRuntime::snapshot()
DurableRuntime::commit_revision(...)
DurableRuntime::checkpoint()
DurableRuntime::compact_obsolete_generations()
```

The lower-level `RuntimeRevisionCell::commit_revision_durable` is now crate-private. This prevents normal callers from supplying an arbitrary durability implementation/store and accidentally mismatching durable and runtime heads.

On restart:

```text
open manifest
-> decode exact checkpoint Revision
-> scan committed WAL tail
-> replay logical tail
-> rebuild PhysicalStore
-> rebuild maintained materializations
-> create fresh RuntimeRevisionCell lineage
```

Stable row handles, indexes, physical layouts and process-local root ids remain reconstructible and are still not checkpoint/WAL semantic authority.

### 3.8 Checkpoint uncertainty remains fail-stop

`RuntimeRevisionCell::checkpoint_durable` holds the writer publication lock while checkpoint rotation occurs.

Before I/O it verifies:

```text
durable_store.durable_head == live RevisionId
```

If rotation fails after entering the durability operation, the runtime transitions conservatively to `RecoveryRequired`; it does not continue serving while checkpoint publication outcome may be uncertain.

## 4. Hostile falsification

### 4.1 Full Revision roundtrip

A checkpoint test includes:

- schema symbols;
- recursive `Mu/Var` type expression;
- capability;
- field;
- Bag relation;
- structural equivalence;
- subtype inclusion;
- pinned equality modules;
- lifecycle state;
- carrier, field value and relation data.

Observed: encode → decode → `Revision::build` produces exact `Revision` equality.

### 4.2 Checkpoint + WAL tail restart

Create durable runtime at R500, commit R501 without checkpoint rotation, drop process owner, reopen directory.

Observed:

```text
checkpoint base = R500
WAL durable head = R501
reopened runtime = R501
physical RowStore rebuilt = expected rows
maintained Scan rebuilt = expected rows
```

### 4.3 Rotation then new tail

Commit R511, checkpoint/rotate to generation 2 at R511, then commit R512.

Observed after restart:

```text
checkpoint generation 2 = R511
WAL-2 tail = R511 -> R512
runtime head = R512
```

### 4.4 Unpublished orphan generation

An extra checkpoint/WAL generation plus torn `pending-manifest` is placed in the directory without a final manifest.

Observed:

```text
open -> still generation 1
next rotation -> generation 3
```

The orphan generation never becomes authority and does not block progress.

### 4.5 Published corruption does not silently roll back

After publishing generation 2, its checkpoint is corrupted.

Observed: reopen returns corruption. It does **not** fall back to generation 1 and silently lose later durable history.

### 4.6 Missing published WAL

Delete the WAL referenced by an otherwise valid published manifest.

Observed: reopen returns `published WAL segment is missing`; it does not create a new empty WAL.

### 4.7 Compaction

After generation 2 publication, compaction removes generation 1 files and directory-fsyncs. Reopen from generation 2 remains valid.

## 5. Verification

Final successful commands after the last production source change:

```text
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets --release
cargo build --workspace --release
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps
RUSTFLAGS='-C overflow-checks=yes' cargo test --workspace --all-targets --release
```

All **PASS**.

The first combined release command and first overflow-check command hit the external 120-second command limit while compiling. Both were rerun separately on the warmed target and completed successfully; no test failure was observed.

Workspace metrics:

```text
273 declared tests
70 kernel-plan tests
65 kernel-query tests
20 kernel-durability tests
20 workspace crates
34,177 Rust LOC
0 external Cargo sources
0 unsafe occurrences under crates/
```

Evidence: `evidence/pass34/`.

## 6. Checklist Pass34

### Closed exactly in this cycle

1. ✅ Exact durable base `Revision=(S,Γ,M)` checkpoint codec for all current schema/model/value forms.
2. ✅ Checkpoint decode revalidates through `Revision::build` and the pinned `SemanticRegistry`.
3. ✅ CRC/version/length framing for checkpoint files.
4. ✅ Immutable generation manifest as durable authority selector.
5. ✅ Prerequisite directory fsync before manifest publication.
6. ✅ Pending-manifest fsync + atomic rename + publication directory fsync.
7. ✅ WAL generation rotation to a new checkpoint base.
8. ✅ Orphan/unpublished generation does not become authority or block the next rotation.
9. ✅ Active manifest may not silently recover through a missing WAL.
10. ✅ Published checkpoint corruption is surfaced rather than hidden by fallback to older history.
11. ✅ Generation compaction + directory fsync.
12. ✅ Durable store enforces `prepare.source_revision == durable_head`.
13. ✅ Checkpoint rotation enforces `checkpoint RevisionId == durable_head`.
14. ✅ `DurableRuntime` binds one runtime publication cell to one durable store/head.
15. ✅ Mainline durable commit no longer exposes arbitrary caller-selected durability backend/store.
16. ✅ Restart owner performs checkpoint decode + WAL scan/replay + physical/materialization rebuild.
17. ✅ Checkpoint rotation failure is treated conservatively as recovery-required.

### Closed from Pass33 OPEN

1. ✅ Exact durable base checkpoint encoding and installation.
2. ✅ Parent-directory fsync/rename + checkpoint/segment manifest protocol.
3. ✅ WAL generation rotation and obsolete-generation compaction; active WAL is never truncated in place.
4. ✅ Unified restart/recovery owner for normal durable runtime operation.

### Previous OPEN narrowed

1. 🟨 Historical Pass26 `WAL/recovery/durable materializations/crash tests`: WAL, exact checkpoints, generation manifests, restart replay and materialization **rebuild** are now integrated. The item remains OPEN because materialization configuration itself is not durable authority and real process/power-loss crash testing is not yet closed.
2. 🟨 `CommitDurabilityUncertain` supervision: normal restart is now one `DurableRuntime::open` operation, but automatic process supervisor/retry policy remains outside this crate.

## 7. Historical Pass26 OPEN backlog still active

1. ⬜ Generic Text/F64 maintained TopK order-statistics.
2. ⬜ I64 TopK constant-factor gap.
3. ⬜ Group/TopK as typed-batch producers.
4. ⬜ Maintained I64 Group constant-factor gap.
5. ⬜ Indexed generic/Text Group.
6. ⬜ Persisted Text/F64/Bool/entity indexes + planner.
7. ⬜ Nested/multiway/mixed-key joins.
8. ⬜ Remaining physical layouts + OrderedView/pagination.
9. ⬜ WAL/recovery/durable materializations/crash tests — production WAL + exact checkpoint + generation restart are integrated; durable materialization config and real crash/power-loss validation remain OPEN.
10. ⬜ Transaction repair runtime, distribution, formal mechanization.
11. ⬜ Semantic indexing/canonical-key strategy for generic maintained Join — Agent-2 R&D VERIFIED; production integration OPEN.

## 8. New / remaining OPEN after Pass34

1. ⬜ Real process/filesystem/power-loss crash matrix on target filesystems; current tests exercise byte corruption, torn WAL tails and generation/orphan states but not actual machine/process kill points.
2. ⬜ Cross-platform durability contract for directory `fsync`/rename semantics, especially Windows and network filesystems.
3. ⬜ Durable materialization configuration/spec authority; restart currently receives `RuntimeMaterializationSpec[]` from the caller and rebuilds state from it.
4. ⬜ Durable revision-DAG / branch+merge parent representation.
5. ⬜ Client transaction/idempotency identity for COMMIT-before-ACK ambiguity.
6. ⬜ Durable schema/Γ/lifecycle/field mutation records. The checkpoint codec can persist such states, but the WAL mutation class remains relation-data only.
7. ⬜ Checkpoint format migration tooling beyond current version rejection.
8. ⬜ Checkpoint streaming/chunking; current correctness-first codec buffers the full checkpoint and enforces a 512 MiB hard limit.
9. ⬜ Rebuild policy/performance for secondary indexes and alternate physical layout replicas after restart.
10. ⬜ Candidate runtime construction remains clone-heavy; COW/persistent roots remain OPEN.
11. ⬜ `RwLock`/durability mutex poisoning policy and external supervisor behavior.
12. ⬜ Group commit, async durability, replication/consensus.
13. ⬜ CRC-32C is accidental-corruption detection, not an adversarial MAC/authentication mechanism.
14. ⬜ Crash-safe garbage collection has been structurally designed as delete-obsolete + directory fsync, but needs actual kill-point falsification before claiming filesystem-level proof.
15. ⬜ Durable store depends on the exact semantic module implementations being available in the supplied `SemanticRegistry`; module binaries/registry deployment are not persisted in the checkpoint.

## 9. Recommended next pass

The logical durability authority chain is now complete enough that the next pass should be **falsification**, not another large feature integration:

```text
Pass35
  process-kill / filesystem crash harness
  + checkpoint rotation kill points
  + COMMIT-before-ACK restart scenarios
  + reopen/repair supervisor contract

then
  close remaining durability defects found by the crash matrix

then
  either durable schema/Γ mutation class
  or Agent-2 production semantic-index integration
```

Do not start Agent-2 integration in parallel with crash-fault work; the current checkpoint/WAL owner should first survive hostile restart testing as one stable substrate.
