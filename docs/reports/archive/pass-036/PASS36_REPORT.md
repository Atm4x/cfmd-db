# PASS36 REPORT — durable client identity + durable runtime metadata + recovery supervisor

Status: **VERIFIED** on Rust **1.98.1**.

Pass36 deliberately groups one coherent durability-control-plane block instead of counting individual tests or fault cases as separate solved problems. Pass35 proved that a committed target is recoverable after `COMMIT durable -> process killed before publish/ACK`, but the external caller still could not name the original transaction, runtime reopen still trusted caller-supplied materialization specs, and `RecoveryRequired` still required manual orchestration.

## 1. Problem

Three operational correctness gaps remained on top of the verified WAL/checkpoint/manifest protocol:

1. **ACK-loss ambiguity.** After durable COMMIT followed by process death before ACK, recovery knew the committed revision but a retry could not prove that it was the same client transaction rather than a new request.
2. **Materialization configuration was not durable authority.** `DurableRuntime::open` still accepted materialization specs from the caller, so restart correctness depended on an external argument matching the configuration that existed before the crash.
3. **Fail-stop recovery had no normal owner.** `RuntimeRecoveryRequired` correctly stopped service, but callers had to manually drop/reopen/retry and interpret an uncertain COMMIT outcome.

These are one durable control-plane problem: recovery needs enough persistent identity/configuration to reconstruct both *what committed* and *what runtime should be rebuilt*, and one process-level owner must apply that information consistently.

## 2. Hypothesis

Make the durable generation authoritative for the missing control-plane state without promoting reconstructible physical state to semantic authority.

```text
published generation N
├─ checkpoint-N.cfdp      exact Revision=(S,Γ,M)
├─ wal-N.cfmw             committed logical tail
├─ metadata-N.cfdm        materialization specs + committed client tx outcomes
└─ manifest-N.cfmf        checksums/identity for the published generation
```

Every client commit carries a nominal `ClientTransactionId`:

```text
(tx_id, source_revision, target_revision, semantic revision, logical mutations)
```

The same `tx_id` may be retried only for the same committed target. A different target is a protocol conflict.

A `DurableRuntimeSupervisor` owns reopen/retry orchestration:

```text
commit(tx_id)
  -> success
  OR uncertain/recovery-required
      -> discard fail-stopped runtime
      -> reopen published durable generation
      -> query tx_id outcome
      -> already committed: return idempotent outcome
      -> unknown: retry exactly the same tx_id
```

## 3. Implementation

Production source changes:

```text
crates/kernel-types/src/lib.rs
crates/kernel-durability/Cargo.toml
crates/kernel-durability/src/lib.rs
crates/kernel-durability/src/metadata.rs   (new)
crates/kernel-durability/src/store.rs
crates/kernel-plan/src/lib.rs
Cargo.lock
```

No external Cargo dependency was added.

### 3.1 Durable client transaction identity

`kernel-types` now defines `ClientTransactionId(u128)`.

`DurableRevisionDescriptor` includes the client transaction ID in the checksummed PREPARE payload. The WAL scanner maintains an exact committed transaction map and rejects conflicting reuse:

```text
same tx_id + same prepare/target     -> idempotent
same tx_id + different prepare       -> protocol error
same tx_id committed to other target -> protocol error
```

`DurableRevisionStore::transaction_outcome` and `DurableRuntime::transaction_outcome` expose only the semantic result needed for safe retry:

```text
Unknown
Committed { target_revision }
```

The committed transaction ledger is folded into every checkpoint generation metadata file, so checkpoint rotation and obsolete-generation compaction do not erase retry identity.

### 3.2 Durable generation metadata

Pass36 adds a versioned `metadata-N.cfdm` file. It contains:

- exact maintained materialization IDs + `RelExpr` specifications;
- committed `ClientTransactionId -> RevisionId` outcomes carried across checkpoint rotation.

The metadata codec is deterministic and bounded. It explicitly encodes every current `RelExpr` variant and its aggregate/order descriptors rather than serializing Rust memory layout.

Metadata participates in the same authority protocol as checkpoint/WAL:

```text
write+fsync checkpoint
write+fsync new WAL
write+fsync metadata
fsync directory prerequisites
write+fsync pending manifest
rename final manifest
fsync directory publication
```

Manifest format v2 binds `metadata_crc32c` in addition to checkpoint CRC. Missing/corrupt manifest-referenced metadata is corruption, never an implicit empty configuration.

Compaction treats metadata as a generation artifact and never deletes the active generation.

### 3.3 Reopen no longer trusts caller materialization specs

`DurableRuntime::create` derives the durable materialization specification set from the validated runtime bundle and persists it in generation metadata.

`DurableRuntime::open(directory, registry)` no longer accepts materialization specs. It obtains them from the published durable generation, then performs the existing:

```text
checkpoint decode/Revision::build
-> WAL committed-prefix scan/replay
-> PhysicalStore rebuild
-> maintained materialization rebuild
-> fresh process-local runtime lineage
```

The semantic registry is still an external deployed implementation registry whose pinned digests must match the durable `SemanticContext`; packaging those implementation binaries is explicitly still open.

### 3.4 Idempotent runtime commit outcome

`DurableRuntime::commit_revision(tx_id, request)` first checks the durable outcome ledger:

- unknown ID: run the ordinary `prepare -> durable PREPARE -> seal -> durable COMMIT -> publish` path;
- already committed to the requested target: return `AlreadyCommitted` without applying data again;
- committed to another target: return `TransactionIdConflict`.

This is semantic idempotency, not merely duplicate WAL-frame acceptance.

### 3.5 Process-level recovery supervisor

`DurableRuntimeSupervisor` owns the directory, cloned semantic registry and current optional runtime.

For `snapshot` and `commit_revision`, a fail-stopped `RecoveryRequired` runtime is discarded and reopened from durable authority. For an uncertain COMMIT, the supervisor queries the same `ClientTransactionId` after reopen and either:

- returns `AlreadyCommitted` for the exact target;
- reports a target conflict;
- retries the request with the exact same transaction ID if no commit exists.

Checkpoint recovery errors are now preserved as `DurableRuntimeCheckpointError::Recovery(RuntimeRecoveryError)` instead of being collapsed into an unrelated generic poison error.

## 4. Hostile falsification

The relevant hostile cases are integrated into the normal test suite.

### 4.1 ACK lost across real process death

The existing Pass35 subprocess kill remains at:

```text
durable COMMIT complete
runtime root not published
ACK absent
parent kills child
```

Pass36 reopens through `DurableRuntimeSupervisor`, retries the **same** client transaction ID, and receives `AlreadyCommitted`. The semantic mutation is not applied twice.

### 4.2 Checkpoint + compaction must not forget transaction identity

A transaction commits, then the runtime checkpoints and compacts obsolete generations. After reopen:

- transaction outcome is still committed;
- retry with the same ID/target is idempotent;
- reuse of the same ID for a different target is rejected.

### 4.3 Materialization specification is recovered from durable authority

The same reopen test does not provide materialization specs to `open()`. The maintained materialization is reconstructed from `metadata-N.cfdm` and produces the expected result.

### 4.4 Fail-stopped runtime supervisor

A runtime is deliberately forced to `RecoveryRequired`; the supervisor discards/reopens it and commits through the recovered owner. This verifies that fail-stop is now an executable operational path rather than only an error state.

### 4.5 Metadata authority corruption

A published manifest whose referenced metadata sidecar is missing/corrupt is rejected. The implementation does not silently recreate an empty materialization/transaction ledger.

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
287 declared tests
74 kernel-plan tests
65 kernel-query tests
30 kernel-durability tests
20 crates
35,711 Rust LOC
0 external Cargo sources
0 unsafe
```

Additional repeated evidence: the client-id/checkpoint retry case, supervisor recovery case and real subprocess COMMIT-before-publish retry were each repeated **10x**; all runs passed. Raw output: `evidence/pass36/DURABLE_CONTROL_STRESS.log`.

## 6. Problem ledger

### Closed exactly in Pass36

1. ✅ **Durable client retry ambiguity after COMMIT-before-ACK.** `ClientTransactionId` is now part of the durable PREPARE identity, committed outcomes survive WAL/checkpoint/compaction/restart, same-ID same-target retry is idempotent, and same-ID different-target reuse is rejected.
2. ✅ **Caller-authoritative materialization configuration on restart.** Materialization specs are now generation metadata selected by the published manifest; `DurableRuntime::open` no longer accepts trusted caller specs.
3. ✅ **Manual `RecoveryRequired` / uncertain-COMMIT restart orchestration.** `DurableRuntimeSupervisor` now owns reopen, outcome resolution and safe same-ID retry for the normal durable runtime path.

### Historical Pass26 OPEN backlog still active

1. ⬜ Generic Text/F64 maintained TopK order-statistics.
2. ⬜ I64 TopK constant-factor gap.
3. ⬜ Group/TopK as typed-batch producers.
4. ⬜ Maintained I64 Group constant-factor gap.
5. ⬜ Indexed generic/Text Group.
6. ⬜ Persisted Text/F64/Bool/entity indexes + planner.
7. ⬜ Nested/multiway/mixed-key joins.
8. ⬜ Remaining physical layouts + OrderedView/pagination.
9. ⬜ WAL/recovery/durable materializations/crash tests — relation-data WAL, checkpoint/manifest/restart, Linux process-kill matrix, durable materialization specs and client retry identity are integrated; machine power-loss, cross-filesystem semantics, semantic-module deployment and wider mutation classes remain OPEN.
10. ⬜ Transaction repair runtime, distribution, formal mechanization.
11. ⬜ Semantic indexing/canonical-key strategy for generic maintained Join — Agent-2 R&D VERIFIED; production integration OPEN.

### Remaining / newly clarified OPEN after Pass36

1. ⬜ **Machine power-loss** validation; process kill cannot prove volatile controller/cache persistence.
2. ⬜ Cross-platform/filesystem durability contract: Windows, network filesystems, FUSE and equivalent rename/directory-sync semantics.
3. ⬜ WAL mutation classes for schema/Γ/lifecycle/field changes. Current tail remains relation-data under unchanged pinned semantic context.
4. ⬜ Durable revision DAG / branch+merge ancestry and parent encoding.
5. ⬜ Durable format migration tooling. Pass36 deliberately advances WAL mutation codec to v2 and manifest to v2; automatic migration from older durable stores is not claimed.
6. ⬜ Streaming/chunked checkpoint and metadata codecs; current implementations buffer bounded payloads in memory.
7. ⬜ Client transaction outcome retention/GC policy. Correctness-first metadata currently retains committed IDs indefinitely.
8. ⬜ Durable materialization **configuration evolution** (add/drop/change as a transactional control-plane operation). Initial/current config is durable; online config mutation is not yet a first-class transaction.
9. ⬜ Rebuild policy/performance for secondary indexes and alternate layout replicas after recovery.
10. ⬜ Candidate runtime roots remain clone-heavy; COW/persistent roots remain OPEN.
11. ⬜ Generic Rust lock-poison policy beyond the durable supervisor's normal `RecoveryRequired` path.
12. ⬜ Group commit, async durability and replication/consensus.
13. ⬜ CRC-32C provides accidental-corruption detection, not authentication/MAC against malicious durable-store modification.
14. ⬜ Semantic-module implementation binaries/registry deployment are still external to the durable store; the store durably pins their digests/contracts, not executable implementations.
15. ⬜ Power-loss-safe proof for manifest rename before directory fsync and crash-safe GC on target production filesystems.

## 7. Result / next direction

Pass36 closes the remaining **normal single-node relation-data durability control plane** around ACK loss, runtime configuration and restart orchestration. It does not claim that every future durable feature is finished.

The next large architectural choice is now cleaner:

```text
A. widen durability semantics
   -> schema/Γ/lifecycle transactions
   -> revision DAG ancestry
   -> format migration / config evolution

or

B. return to data-plane performance
   -> integrate Agent-2 canonical semantic indexes
   -> generic Join / Group / TopK
```

Given the request to finish durability first, the recommended next pass is a **general durable revision-change protocol** for schema/Γ/lifecycle/field transitions, designed on top of existing `kernel-transport` rather than serializing ad-hoc host mutations.
