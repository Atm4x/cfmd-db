# Pass63 hostile review — R&D Program7 delta-native durability

Status: **R&D PARTIAL / NOT INTEGRATED**.

## Candidate claim

Program7 replaces the relation-data `DurableTransactionIntent::Exact` full target snapshot with `RelationDataExact { source_revision, target_revision, semantic_revision, relation_mutations, semantic_modules }`. This is intended to make small relation-data transaction intent/ledger storage scale with the delta rather than the complete target Revision.

The package itself is internally consistent and its patch dry-runs against Pass63. Its v5 mutation codec, v4 metadata compatibility and space diagnostic are technically plausible. The design also correctly leaves full-revision replacement on complete target bytes.

## Hostile counterexample

The candidate does **not** preserve the existing exact retry identity contract under the current API/identity model.

`RevisionTransitionRequest` contains both:

```text
target_revision: &Revision
mutations: &[RevisionRelationMutation]
```

and `RevisionId` is nominal, not content-addressed. On an original commit, `prepare_revision()` proves that the supplied target content is exactly the result of applying the mutation set to the current source revision. After later commits + checkpoint/compaction, however, Program7's retained relation-data intent contains only the old source/target IDs plus the delta; it no longer contains enough information to distinguish a retry carrying the same IDs/delta from one carrying a different untouched target state.

The candidate runtime retry path checks the retained intent **before** running `prepare_revision()`. It reconstructs the requested `RelationDataExact` using the *stored original source RevisionId*, the caller's target RevisionId and the caller's mutations. Therefore a caller can present:

```text
same ClientTransactionId
same source/target RevisionId lineage
same relation delta
same Γ / module deployment
DIFFERENT target Revision content outside the delta
```

and collide with the retained intent.

## Reproduction

A temporary Pass63 copy was patched with Program7. The existing historical exact-intent regression was strengthened so its post-checkpoint conflicting retry uses `commit_revision` with the original relation mutations instead of switching API surface to `replace_revision`.

Sequence:

1. commit relation-data transaction `T` from revision 865 to 866;
2. commit a later head;
3. checkpoint + compact obsolete generations;
4. reopen;
5. build a different valid `RevisionId(866)` whose relation content is `[999]` rather than the original target;
6. retry `T` through `commit_revision` with the original delta.

Expected under the existing exact-intent contract: `TransactionIdConflict`.

Observed with Program7: the assertion fails because the relation-data intent compares equal before target-content validation.

Evidence: `evidence/pass63/PROGRAM7_HOSTILE_EXACT_RETRY_COUNTEREXAMPLE.txt` (exit code 101).

## Why this is architectural

This cannot be fixed merely by another delta equality check. Under the current semantics, an exact durable retry must distinguish arbitrary target Revision contents even when nominal RevisionIds and the typed relation delta coincide. Once the historical source/target snapshots have been compacted, a finite delta tuple alone does not contain that information.

Clean repair directions are broader than the submitted patch:

1. make relation-data transaction requests delta-authoritative, so runtime derives the target Revision internally and the caller cannot supply an independent target content; or
2. introduce a durable content identity/root certificate with an explicitly justified collision/trust contract; or
3. retain an exact historical source/target/change witness sufficient to revalidate the target without snapshot-per-transaction duplication.

Simply adding a non-injective checksum while continuing to call the contract mathematically exact would violate the CFMD semantic discipline.

## Result

Program7's **space diagnosis is accepted**: the current production relation-data idempotency ledger duplicates snapshot-sized target bytes and is an important durability frontier.

Program7's **production closure is rejected** because its replacement weakens exact client-intent identity. No Program7 production source is included in Pass63.

The R&D branch should revise the authority/API boundary and rerun this exact post-compaction counterexample before resubmission.
