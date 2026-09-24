# PASS81 CHECKPOINT I — DURABLE REWRITE INTENT

Status: **DEBUG+CLIPPY VERIFIED**.

Closed in this checkpoint:

1. `RevisionCommitDescriptor` is the single in-memory owner of relation Rewrite intent metadata; prepared/sealed transitions delegate to it instead of carrying a parallel map.
2. Added exact durable `RelationRewriteExact` identity: source/target revision, pinned semantic revision, canonical typed relation deltas, and canonical `(relation, RewriteSpecId, RewriteLawSetId)` entries.
3. WAL mutation codec is v6. Relation Rewrite prepare records use a distinct tag; v5 relation-data records remain readable.
4. Durable store metadata codec is v6 and persists committed Rewrite intents across checkpoint rotation/compaction/restart; metadata v5 remains readable.
5. `commit_derived_relation_rewrites` derives target content from the authoritative source, routes through the existing Rewrite/VMF/DTC publication boundary, and uses Rewrite identity for idempotency.
6. Retry after checkpoint+compaction+reopen with the same transaction ID and same delta/spec is `AlreadyCommitted`; same endpoint/delta but a different RewriteSpec/law set is `TransactionIdConflict`.

Hostiles:
- WAL v6 relation Rewrite roundtrip;
- mutation codec v5 relation-data backward decode;
- metadata roundtrip for `RelationRewriteExact`;
- restart/idempotency conflict on endpoint-equivalent but intent-distinct Rewrite.

Verification:
- `cargo fmt --all -- --check` PASS;
- `cargo check --workspace --all-targets` PASS;
- `cargo test --workspace --all-targets`: **544 passed / 0 failed / 8 ignored**;
- `cargo clippy --workspace --all-targets -- -D warnings` PASS;
- **552 declared tests**;
- no lint suppressions added.

Not claimed: durable complement capsules/retention enforcement, relational writable Project/Filter/Join synthesis, REIC consumption of Rewrite law certificates.
