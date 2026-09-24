# IMPLEMENTATION REPORT — PASS92

## Scope
Pass92 closes the durable causal-DAG lifecycle gap (#8) and advances the ordered replication authority boundary (#16), while leaving #21/#22 untouched under external R&D ownership.

## Changed production files

### `crates/kernel-durability/src/replication.rs` — new
Implements `ReplicationAuthorityJournal`, an append-only CRC-protected and fsync-backed log for non-head REIC effects. Adds:
- `ReplicaId`;
- `ReplicationBranchId`;
- `DurableSequencerOrder`;
- `ReplicatedEffectEnvelope`;
- `ReplicationBranchHead`;
- `ReplicationIngestOutcome`;
- `replicated_effect_id` / `replicated_origin`.

The journal keeps branch heads, replicated revision frontiers, sequencer-slot ownership and highest accepted sequencer epochs. Torn final frames are truncated; complete corrupted frames fail closed.

### `crates/kernel-durability/src/store.rs`
Integrates the replication journal into store create/open lifecycle. Adds store APIs:
- `replication_branch_head`;
- `replicated_effect_record`;
- `replication_journal_path`;
- `durably_ingest_replicated_effect`;
- `durably_retire_replication_branch`;
- `replicated_branch_effect_ideal`.

Admission computes the authoritative causal cut from local + replicated frontiers, including multi-parent resolution cuts. Semantic module packages are execution-authorized through the Pass91 deployment boundary before an effect is durably admitted. Reopen revalidates every persisted replicated effect against the recovered local authority.

Local effect allocation is now permanently bounded to the low 64-bit namespace so replicated origin namespaces cannot alias local event identities.

### `crates/kernel-durability/src/lib.rs`
Exports the replication authority types and adds `DurableTransactionIntent::source_revision()`. `DurableRevisionEffectRecord::validate_identity()` now also verifies that source-bearing exact intents agree with the record source revision.

### `crates/kernel-durability/src/metadata.rs`
Only widens the existing transaction-intent codec helpers to crate visibility so the replication journal serializes the exact same canonical intent format instead of introducing another codec/ontology.

## Authority invariants
1. Main transaction WAL remains linear and source=`durable_head`; Pass92 does not permit non-head PREPAREs there.
2. Replicated effects use the same durable REIC record and exact transaction intent as local effects.
3. Branch ingestion cannot publish to readers or mutate `durable_head`.
4. Causal prerequisites must equal the authoritative source/multi-parent cut, not merely be a subset.
5. Local and replicated effect IDs are namespace-disjoint by construction.
6. `OpaqueNonConfluent` remote effects require externally ordered admission; arrival order is never authority.
7. Sequencer epochs are monotone fences and durable replay reconstructs the fence.
8. Branch retirement is durable and irreversible for that branch identity.

## Validation
Final source state passed:
- `cargo fmt --all -- --check`;
- `cargo check --workspace --all-targets`;
- `cargo clippy --workspace --all-targets -- -D warnings`;
- `cargo test --workspace --all-targets`.

Result: **676 declared tests, 0 failures, 8 ignored**.

## Non-claims
Pass92 is not a complete consensus implementation. `DurableSequencerOrder` is an already-established ordering witness; this crate currently verifies durable uniqueness/fencing/replay but not signatures, membership, quorum votes, leader election, network transport or quorum-loss recovery. Certified-confluent anti-entropy admission also remains future work because production has no durable confluence certificate attached to these effects yet.
