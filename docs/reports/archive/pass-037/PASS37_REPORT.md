# PASS37 REPORT — general durable revision changes + online durable materialization configuration

Status: **VERIFIED** on Rust **1.98.1**.

Pass37 intentionally closes a small number of real durability problems rather than counting individual tests as separate problems. Pass36 completed the normal relation-data durability control plane, but the durable mutation vocabulary still could not represent schema/Γ/lifecycle/field changes and durable materialization configuration could not be changed online.

## 1. Problem

Three concrete correctness/authority gaps were addressed.

1. **The WAL mutation vocabulary was relation-data-only.** A validated target `Revision=(S,Γ,M)` could change schema, pinned semantic environment, lifecycle/carriers or fields, but there was no durable mutation class capable of committing that target through the normal WAL/recovery path.
2. **Materialization configuration was durable but static.** Pass36 made the current `MaterializationId + RelExpr` registry part of durable generation authority, but add/drop/change still had no atomic online operation tying the new durable configuration to the matching rebuilt runtime root.
3. **A retry could over-trust nominal target identity.** A committed `ClientTransactionId` and matching `RevisionId` were not enough to justify returning idempotent success if the caller supplied a different current target Revision with the same nominal ID.

## 2. Design

### 2.1 Typed durable semantic-change classes

`DurableRevisionDescriptor` now carries an explicit `DurableRevisionChange`:

```text
RelationData {
    semantic_revision,
    relation_mutations,
}

FullRevision {
    encoded_target_revision,
}
```

`RelationData` remains the compact fast path for ordinary relation mutations under an unchanged semantic context.

`FullRevision` is the correctness path for a general semantic revision replacement. It stores an exact canonical checkpoint-format image of the target `Revision=(S,Γ,M)` inside the checksummed PREPARE payload. This is intentionally semantic state, not physical execution state.

Recovery decodes the full target through the normal revision codec and validation boundary. The target ID in the frame/descriptor must agree with the decoded Revision. Physical rows, stable handles, indexes and maintained results are then reconstructed.

### 2.2 Runtime full-revision transition

`kernel-plan` adds `FullRevisionTransitionRequest` and a full-revision prepare path.

The candidate is rebuilt from the validated target Revision using deterministic recovery row-store layouts and the currently durable materialization specification registry. Therefore a schema/Γ change that makes a materialization invalid fails during prepare, before WAL PREPARE publication.

The publication law is unchanged:

```text
build full candidate
-> durable PREPARE(full semantic target)
-> seal current runtime root
-> durable COMMIT
-> infallible whole-root publish
```

Successful publication reports `RuntimePublicationEffect::Rebuilt`, distinguishing it from an incremental relation-data delta.

### 2.3 Online durable materialization configuration

A new private prepared/sealed materialization-configuration transition rebuilds all desired maintained states against the current authoritative Revision and physical rows before publication.

`DurableRuntime::reconfigure_materializations` performs:

```text
validate desired spec registry
-> prepare rebuilt materializations
-> seal current runtime root
-> rotate checkpoint generation with new durable specs
-> publish new runtime root
```

The existing immutable generation/manifest protocol is reused. If durable generation publication is uncertain, the runtime fail-stops. `DurableRuntimeSupervisor` reopens durable authority and retries; an already-published identical spec registry returns `AlreadyApplied`.

This is correctness-first and checkpoint-based. It is not yet an incremental configuration-WAL optimization.

### 2.4 Retry target-content hardening

When the committed ledger reports the same `ClientTransactionId` and target `RevisionId`, `DurableRuntime` now also compares the currently committed Revision with the caller-supplied target. Different Revision contents produce `TransactionTargetMismatch` instead of `AlreadyCommitted`.

This closes the immediate false-idempotency path without pretending that the current transaction-outcome ledger is already a complete retained content-addressed history. Exact historical intent identity after later heads advance remains explicitly OPEN.

### 2.5 Mutation codec compatibility

Mutation codec advances from v2 to v3 because PREPARE now has an explicit change tag.

The decoder still accepts v2 relation-data PREPARE payloads and maps them to `DurableRevisionChange::RelationData`. This is deliberate payload backward compatibility, not a claim of universal automatic durable-store migration.

## 3. Hostile falsification

Integrated cases verify the semantic boundaries rather than merely exercising serialization:

- a full revision changes schema revision, semantic environment revision, a pinned equality implementation, lifecycle/carrier membership, fields and relation rows in one commit;
- restart recovers the exact full target and rebuilt maintained state;
- same transaction ID + same nominal RevisionId + different target content is rejected;
- materialization registry can atomically replace one materialization with another, survives restart, and repeated identical reconfiguration is idempotent;
- an invalid materialization reconfiguration is rejected before durable publication and both live/reopened configurations remain unchanged;
- mutation-v2 relation-data payload remains readable under the v3 decoder.

The pre-existing Pass35 subprocess crash matrix still applies to the common PREPARE/COMMIT ordering used by both mutation classes; Pass37 does not count those killpoints as newly solved problems.

## 4. Verification

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
293 declared tests
78 kernel-plan tests
65 kernel-query tests
32 kernel-durability tests
20 crates
36,756 Rust LOC
0 external Cargo sources
0 unsafe
```

## 5. Problem ledger

### Closed exactly in Pass37

1. ✅ **No durable mutation class for schema/Γ/lifecycle/field changes.** A validated complete target Revision can now commit and recover through a typed `FullRevision` WAL record while physical/materialized state remains reconstructible.
2. ✅ **Durable materialization configuration could not evolve online.** Add/drop/change is now an atomic durable operation tied to checkpoint-generation publication and matching whole-root runtime publication; restart obtains the changed configuration from durable authority.
3. ✅ **False idempotent success for same client transaction ID + same current RevisionId + different current Revision content.** The runtime now reports an explicit target-content mismatch rather than treating nominal ID equality as sufficient.

### Closed from the Pass36 OPEN list

1. ✅ WAL mutation classes for schema/Γ/lifecycle/field changes.
2. ✅ Durable online materialization configuration evolution.

### Partially advanced, still OPEN

1. 🟨 **Durable format migration.** v2 relation-data mutation payloads remain readable under v3. General checkpoint/manifest/metadata migration tooling is still OPEN.

### Historical Pass26 OPEN backlog still active

1. ⬜ Generic Text/F64 maintained TopK order-statistics.
2. ⬜ I64 TopK constant-factor gap.
3. ⬜ Group/TopK as typed-batch producers.
4. ⬜ Maintained I64 Group constant-factor gap.
5. ⬜ Indexed generic/Text Group.
6. ⬜ Persisted Text/F64/Bool/entity indexes + planner.
7. ⬜ Nested/multiway/mixed-key joins.
8. ⬜ Remaining physical layouts + OrderedView/pagination.
9. ⬜ WAL/recovery/durable materializations/crash tests — relation-data WAL, full semantic revision WAL, checkpoint/manifest/restart, Linux process-kill matrix, durable materialization configuration and client retry supervision are integrated; machine power-loss, cross-filesystem assurance, semantic-module deployment and durable DAG history remain OPEN.
10. ⬜ Transaction repair runtime, distribution, formal mechanization.
11. ⬜ Semantic indexing/canonical-key strategy for generic maintained Join — Agent-2 R&D VERIFIED; production integration OPEN.

### Remaining / newly clarified OPEN after Pass37

1. ⬜ Machine power-loss validation; process kill cannot prove volatile controller/cache persistence.
2. ⬜ Cross-platform/filesystem durability contract: Windows, network filesystems, FUSE and equivalent rename/directory-sync semantics.
3. ⬜ Durable revision DAG / branch+merge ancestry and parent encoding. Current `DurableRuntime` is a linear committed-head authority.
4. ⬜ General automatic migration tooling for historical checkpoint/manifest/metadata/mutation format combinations.
5. ⬜ Streaming/chunked checkpoint and metadata codecs; current bounded codecs buffer complete payloads.
6. ⬜ Client transaction outcome retention/GC policy. Correctness-first metadata retains committed IDs indefinitely.
7. ⬜ Exact historical request-content identity for a retry after the live head has advanced beyond the committed transaction. The immediate same-head false-idempotency path is fixed, but the retained ledger is still primarily `ClientTransactionId -> RevisionId`.
8. ⬜ Rebuild policy/performance for secondary indexes and alternate layout replicas after recovery/full-revision replacement.
9. ⬜ Candidate runtime roots remain clone-heavy; COW/persistent roots remain OPEN.
10. ⬜ Generic Rust lock-poison policy beyond the durable supervisor's normal `RecoveryRequired` path.
11. ⬜ Group commit, async durability and replication/consensus.
12. ⬜ CRC-32C provides accidental-corruption detection, not authentication/MAC against malicious durable-store modification.
13. ⬜ Semantic-module implementation artifacts/registry deployment remain external to the durable store. Durable Revision pins digests/contracts, but restart still requires a compatible externally supplied `SemanticRegistry`.
14. ⬜ Formal/power-loss proof for manifest rename, directory fsync and crash-safe GC on target production filesystems.
15. ⬜ Materialization reconfiguration is currently correctness-first checkpoint rotation; an incremental config-WAL form is a performance/operational optimization, not a correctness requirement.
16. ⬜ One atomic transaction cannot yet change both the semantic Revision and the materialization specification registry. Schema migrations that invalidate old queries require an explicit compatible staging sequence (for example drop old materialization -> replace Revision -> add new materialization).

## 6. Result / next direction

For the current **single-head** database model, the functional single-node durability path now covers:

```text
relation data changes
+ full schema/Γ/lifecycle/field revision replacement
+ durable materialization configuration evolution
+ PREPARE/COMMIT + ACK-loss retry
+ checkpoint/manifest generations
+ restart/rebuild
+ process-kill falsification
```

The remaining durability list is dominated by deployment/history/scalability/assurance rather than a missing ordinary commit/recovery path.

Unless hostile review finds a new correctness defect, the next high-value mainline pass should return to the historical data-plane backlog and integrate the already-verified Agent-2 exact semantic key/index design, beginning with generic maintained Join and then Group/TopK.
