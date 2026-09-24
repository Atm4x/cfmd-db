# PASS35 REPORT — real subprocess killpoints + durable-COMMIT-before-publish recovery

Status: **VERIFIED** on Rust **1.98.1**.

Pass35 is deliberately a falsification pass. Pass34 had a complete software durability protocol, but the checkpoint/manifest ordering and COMMIT boundary had only been tested with synthetic torn/corrupt bytes and constructed orphan states. This pass subjects the actual implementation to externally killed subprocesses at named durability boundaries.

## 1. Problem

The remaining durability claim was stronger than the evidence.

Pass34 established:

```text
checkpoint-N + wal-N + manifest-N
PREPARE -> seal -> COMMIT -> publish
```

but did not yet demonstrate what a fresh process sees if the writer dies at the boundaries between those steps.

The critical questions were:

1. does kill after PREPARE expose an uncommitted target?
2. does kill after COMMIT but before ACK recover the target?
3. which checkpoint generation is authoritative at each manifest publication stage?
4. can kill during obsolete-generation GC damage the active generation?
5. if the process dies after durable COMMIT but before runtime root publication, does restart recover from durable authority rather than the dead process's stale in-memory root?

## 2. Hypothesis

Add a **private fault-injection seam**, not a public testing capability.

Normal production operations use a zero-cost semantic `NoStoreFault` hook. Test workers use the same internal code path with a blocking fault hook:

```text
worker reaches named durability stage
-> writes test-only ready marker
-> blocks
parent observes marker
-> Child::kill()
-> waits for death
-> opens durable store/runtime from a fresh process context
-> checks recovered semantic authority
```

No test hook, marker or crash-point identity participates in manifest/WAL/checkpoint authority.

## 3. Implementation

Production/test source changed in:

```text
crates/kernel-durability/src/store.rs
crates/kernel-plan/src/lib.rs
```

Documentation/evidence added in:

```text
PASS35_REPORT.md
IMPLEMENTATION_REPORT_pass35.md
PASS35_CRASH_MATRIX.md
evidence/pass35/CRASH_STRESS.log
CFMD_IDEAL_DB_SPEC.md
ARCHITECTURE.md
```

No external Cargo dependency was added.

### 3.1 Private checkpoint/manifest/GC killpoints

`DurableRevisionStore` now factors the existing implementation through an internal fault hook. Public production methods always pass `NoStoreFault`.

Named internal points cover:

```text
AfterCheckpointSync
AfterWalSync
AfterPrerequisiteDirectorySync
AfterPendingManifestSync
AfterManifestRename
AfterManifestDirectorySync
BeforeCompactionRemove
AfterCompactionRemove
AfterCompactionDirectorySync
```

These do not change the public API or durable format.

### 3.2 Real external process kill

The crash tests do not call normal shutdown or rely on unwinding. A worker blocks after signalling that it reached the requested boundary; its parent calls `std::process::Child::kill()`.

This matters because normal Rust destructors are not allowed to repair or flush state after the selected point.

### 3.3 WAL PREPARE/COMMIT kill boundaries

Two subprocess cases exercise the real `DurableRevisionStore`:

- kill after `durably_prepare` returns: reopen recovers the previous committed revision and zero committed records;
- kill after `durably_commit` returns but before caller ACK: reopen recovers the target revision and the committed descriptor.

### 3.4 Checkpoint/manifest authority matrix

A committed WAL head R91 is checkpoint-rotated from generation 1 to generation 2. The child is killed at every durability stage.

Observed:

```text
after checkpoint sync                -> generation 1 authority
new WAL sync                         -> generation 1 authority
prerequisite directory sync          -> generation 1 authority
pending manifest sync                -> generation 1 authority
manifest rename                      -> generation 2 visible after process kill
publication directory sync           -> generation 2 authority
```

For all pre-rename kills, generation 1's WAL tail still reconstructs R91. No semantic commit is lost merely because checkpoint rotation was interrupted.

The rename-before-directory-fsync result is explicitly **process-kill evidence only**, not a claim about machine power-loss persistence.

### 3.5 Compaction kill matrix

The worker is killed:

- before deleting an obsolete artifact;
- after deleting one obsolete artifact;
- after compaction directory fsync.

A fresh open always selects and validates the active generation. Partial GC may leave or remove obsolete files but does not delete the active generation.

### 3.6 COMMIT durable, runtime publish never happened

The strongest end-to-end falsifier wraps the real `DurableRevisionStore` and kills the worker immediately after `durably_commit` succeeds, inside `RuntimeRevisionCell::commit_revision_durable`, before `sealed.publish()` can execute.

The dead process therefore had:

```text
disk: target revision committed
memory: old runtime root still unpublished
ACK: absent
```

Fresh `DurableRuntime::open` reconstructs the target Revision, physical RowStore and maintained Scan from checkpoint + WAL. This confirms that runtime publication is not recovery authority.

### 3.7 Repeated stress

The crash matrix was repeated after the primary gate:

```text
20x kernel-durability subprocess-kill matrix      PASS
10x COMMIT-durable-before-publish restart         PASS
```

Raw evidence is in `evidence/pass35/CRASH_STRESS.log`.

## 4. Hostile result

No new correctness defect was found in the Pass34 ordering under the tested process-kill model.

The matrix strengthens four architectural claims:

1. PREPARE is not authority.
2. COMMIT is the logical durability point.
3. final manifest publication selects the checkpoint/WAL generation.
4. in-memory root publication is not needed for crash recovery after COMMIT.

It also exposes one intentionally still-open product contract:

```text
COMMIT durable
process dies before ACK
client retries because outcome is unknown
```

Recovery knows the transaction committed, but the client currently has no durable idempotency/transaction key with which to identify that fact. Pass35 does **not** paper over this ambiguity.

## 5. Verification

Final Rust 1.98.1 gate:

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

Metrics:

```text
282 declared tests
72 kernel-plan tests
65 kernel-query tests
27 kernel-durability tests
20 crates
34,625 Rust LOC
0 external Cargo sources
0 unsafe
```

## 6. Problem ledger

### Closed exactly in Pass35

1. ✅ Real **subprocess kill** coverage for durable PREPARE boundary.
2. ✅ Real subprocess kill coverage for durable COMMIT-before-ACK boundary.
3. ✅ Real subprocess kill matrix for checkpoint file sync / new WAL sync / prerequisite directory sync.
4. ✅ Real subprocess kill before final manifest rename.
5. ✅ Real subprocess kill after manifest rename and after publication directory fsync under the current filesystem.
6. ✅ Real subprocess kill during obsolete-generation compaction.
7. ✅ End-to-end `COMMIT durable -> process killed before runtime publish -> restart` recovery.
8. ✅ Repeated crash-harness stress proving the test itself is not a one-shot timing accident.
9. ✅ Crash fault injection remains private and does not add a caller-visible authority escape.

### Closed from Pass34 OPEN

1. ✅ The **process-kill** portion of “real process/filesystem/power-loss crash matrix” is now closed for the current Linux/container filesystem.
2. ✅ Process-kill falsification of obsolete-generation GC is closed for the current filesystem.

### Historical Pass26 OPEN backlog still active

1. ⬜ Generic Text/F64 maintained TopK order-statistics.
2. ⬜ I64 TopK constant-factor gap.
3. ⬜ Group/TopK as typed-batch producers.
4. ⬜ Maintained I64 Group constant-factor gap.
5. ⬜ Indexed generic/Text Group.
6. ⬜ Persisted Text/F64/Bool/entity indexes + planner.
7. ⬜ Nested/multiway/mixed-key joins.
8. ⬜ Remaining physical layouts + OrderedView/pagination.
9. ⬜ WAL/recovery/durable materializations/crash tests — WAL/checkpoint/restart and Linux process-kill matrix are integrated; durable materialization config, cross-filesystem semantics and power-loss validation remain OPEN.
10. ⬜ Transaction repair runtime, distribution, formal mechanization.
11. ⬜ Semantic indexing/canonical-key strategy for generic maintained Join — Agent-2 R&D VERIFIED; production integration OPEN.

### Remaining / newly clarified OPEN after Pass35

1. ⬜ **Machine power-loss** validation. Process kill does not prove storage-controller/cache persistence semantics.
2. ⬜ Cross-platform durability contract: Windows, network filesystems, FUSE and other filesystem rename/directory-sync semantics.
3. ⬜ Durable client transaction/idempotency identity for `COMMIT durable -> ACK lost -> retry`.
4. ⬜ Durable materialization configuration/spec authority; reopen still receives specs from caller.
5. ⬜ Durable revision DAG / branch+merge parent representation.
6. ⬜ WAL records for schema/Γ/lifecycle/field mutations; current WAL class is relation-data only.
7. ⬜ Checkpoint format migration tooling.
8. ⬜ Streaming/chunked checkpoint; current codec buffers the full checkpoint and caps it at 512 MiB.
9. ⬜ Rebuild policy/performance for secondary indexes and alternate layout replicas.
10. ⬜ Candidate runtime roots remain clone-heavy; COW/persistent roots remain OPEN.
11. ⬜ Automatic supervisor policy around poisoned locks / `RecoveryRequired` and retry/restart orchestration.
12. ⬜ Group commit, async durability and replication/consensus.
13. ⬜ CRC-32C is accidental-corruption detection, not authentication/MAC.
14. ⬜ Semantic-module binary/registry deployment is not packaged as durable store authority.
15. ⬜ A process kill **after rename but before directory fsync** observes the new manifest on this filesystem, but this is explicitly not evidence that a sudden power loss at the same point preserves the rename.

## 7. Recommended next pass

The relation-data durability line is now strong enough that the next pass should close the most important operational ambiguity rather than add more filesystem machinery:

```text
Pass36
  durable client transaction / idempotency identity
  + retry-after-ACK-loss semantics
  + recovery query for transaction outcome
  + supervisor contract for RecoveryRequired

then
  choose between schema/Γ durable transaction generalization
  and Agent-2 semantic-index production integration
```

Power-loss/cross-filesystem testing remains important, but it requires an environment capable of actually controlling filesystem/machine failure semantics; pretending that another in-process truncation test proves it would be weaker evidence than the current explicit boundary.
