# Pass68 hostile review — corrected R&D Program7 delta-native durability

Status: **ACCEPTED AND INTEGRATED WITH ADDITIONAL HOSTILE COVERAGE**.

## Prior blocker

Pass63 rejected the first Program7 because the existing `commit_revision` API accepted an independently supplied target `Revision`, while the proposed compact retry identity retained only source/target IDs plus relation delta. After checkpoint/compaction, the same transaction id, same nominal IDs and same delta could therefore be retried with different target content and incorrectly compare equal.

## Corrected authority split

The corrected Program7 does not weaken that API contract.

- `DurableRuntime::commit_revision` remains the legacy independently-supplied-target surface. Its durable idempotency witness remains the complete canonical target revision encoding.
- `DurableRuntime::commit_derived_relation_data` is a new delta-authoritative surface. It accepts only source revision id, target revision id and typed relation mutations. The runtime derives the target `Revision` from the authoritative live source and the pinned semantic registry.
- Compact `DurableTransactionIntent::RelationDataExact` is used only on that derived surface. No independently supplied target content exists there to be omitted from the intent.

This resolves the Pass63 information-theoretic objection by changing the authority boundary rather than pretending a non-injective checksum is exact.

## Hostile review performed in Pass68

The original Pass63 counterexample was rerun through the legacy full-target API after later heads, checkpoint, compaction and reopen. Different target content with the same nominal target id still returns `TransactionIdConflict`.

The corrected compact API was tested through commit -> later head -> checkpoint -> compaction -> reopen:

- exact same derived request returns `AlreadyCommitted`;
- changed delta under the same transaction id conflicts;
- the recovered logical head remains exact.

Pass68 adds a further production hostile not present in the R&D package:

`derived_relation_commit_matches_authoritative_target_and_rejects_stale_source`

It proves both:

1. a target derived internally from `(source, delta)` is exactly the same validated Revision that the existing authoritative relation-transition construction produces; and
2. a fresh transaction naming a stale source revision is rejected before durable publication, remains absent after reopen, and cannot silently commit over a later head.

Freshness under concurrent root movement remains protected by the existing prepared-transition seal: the final writer guard checks both process-local `root_identity` and source `RevisionId` before durable COMMIT.

## Format / space review

Program7 introduces mutation codec v5 and metadata codec v4 while retaining previous decoder paths. Relation-data PREPARE encodes the canonical typed delta once and reconstructs both the durable intent and replay change from those bytes. Full target bytes remain present only for APIs whose client supplies full target authority.

The production regression `relation_data_intent_and_prepare_scale_with_delta_not_target_snapshot` constructs a large unrelated target state plus a one-row mutation and requires both compact PREPARE and committed transaction metadata to remain more than 100x smaller than the canonical full revision encoding. This is structural evidence, not a storage SLA.

## Non-closures

Program7 does **not** close:

- transaction intent/outcome retention or GC;
- streaming/chunked checkpoints or metadata;
- general historical durable-format migration;
- group commit / async durability;
- replication/consensus;
- authenticated durable storage;
- cross-filesystem or power-loss proof obligations.

The committed-transaction ledger remains unbounded in transaction count even though relation-data entries are no longer snapshot-sized.
