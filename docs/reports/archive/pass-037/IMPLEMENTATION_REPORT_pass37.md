# IMPLEMENTATION REPORT — Pass37

## problem

Pass36 made relation-data commits, checkpoint/restart, client idempotency and materialization specification bootstrap durable. Two functional durability gaps remained: semantic revisions that change schema/Γ/lifecycle/fields could not enter the WAL tail, and durable materialization configuration could not be changed online. A retry boundary also over-trusted a matching nominal target RevisionId when the supplied current Revision content differed.

## hypotheses

1. Do not add ad-hoc schema flags to the relation-data record. Use an explicit sum type for durable revision changes.
2. A general semantic migration may be persisted as the exact validated target `Revision=(S,Γ,M)` and rebuilt physically on recovery; physical layout/handle/index state must stay reconstructible.
3. Online materialization reconfiguration can reuse the already-proved immutable checkpoint-generation publication protocol: prepare/rebuild first, durably publish configuration, then infallibly publish the matching runtime root.
4. Nominal RevisionId equality alone is insufficient evidence that the caller supplied the same current target Revision.
5. Compatibility should be explicit: preserve v2 relation-data PREPARE decoding while advancing the mutation codec to v3.

## implementation

### kernel-durability

Added `DurableRevisionChange`:

```text
RelationData { semantic_revision, relation_mutations }
FullRevision { encoded_target_revision }
```

`DurableRevisionDescriptor` now carries this tagged change. Mutation codec v3 encodes the tag. The full-revision payload reuses the canonical revision checkpoint codec and is revalidated on decode. Mutation codec v2 relation-data payloads are still accepted.

`DurableRevisionStore::rotate_checkpoint_with_materializations` allows a validated runtime to publish a new materialization specification registry through the existing checkpoint/WAL/metadata/manifest generation protocol.

### kernel-plan

`RevisionCommitDescriptor` mirrors the semantic split with `RevisionCommitChange::{RelationData, FullRevision}`.

Added `FullRevisionTransitionRequest`. `RuntimeRevisionBundle::prepare_full_revision` rebuilds the physical store and every current materialization against the target semantic Revision before any WAL record is made. It retains runtime lineage and advances only the version.

`DurableRuntime::replace_revision` and `DurableRuntimeSupervisor::replace_revision` route the full target through the same durable PREPARE/seal/COMMIT/publication boundary as ordinary commits.

Publication receipts distinguish incremental results from rebuilds via `RuntimePublicationEffect::{Incremental, Rebuilt}`.

Added prepared/sealed materialization-configuration transition. `DurableRuntime::reconfigure_materializations` validates/rebuilds the desired registry, rotates a checkpoint generation containing the new specs, then publishes the candidate runtime root. Supervisor retry handles uncertain publication by reopening durable authority and recognizing an already-applied identical registry.

Committed transaction retry now checks the supplied target Revision content when the nominal transaction/target IDs match the current committed target. A different target value yields `TransactionTargetMismatch`.

## hostile falsification

The production suite now covers:

- full semantic replacement changing schema revision, Γ implementation, lifecycle/carriers, fields and relation rows;
- exact restart/replay of that target;
- same transaction ID and same RevisionId but different Revision content;
- durable add/drop/change of materialization configuration plus restart;
- invalid configuration rejected before durable publication;
- v2 relation-data PREPARE compatibility under the v3 decoder.

Existing process-kill durability tests exercise the shared PREPARE/COMMIT publication law and remain part of the full workspace gate.

## verification

Rust 1.98.1:

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

Metrics: 293 declared tests; 78 kernel-plan; 65 kernel-query; 32 kernel-durability; 20 crates; 36,756 Rust LOC; zero external Cargo sources; zero `unsafe`.

## result

Pass37 closes three actual problems:

1. the WAL/recovery path now supports a complete validated semantic Revision change, not only relation-data delta under unchanged Γ;
2. durable materialization configuration can evolve online atomically and survives reopen without caller authority;
3. matching transaction/Revision IDs no longer hide a different current target Revision content.

The remaining durability work is mostly durable history/deployment/migration/scalability/assurance: DAG ancestry, semantic implementation packaging, general format migration, outcome-ledger GC, streaming codecs, production-filesystem power-loss validation, COW candidates and distributed/async durability.
