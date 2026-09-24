# CFMD Implementation Report — Pass68

Status: **VERIFIED**
Base: verified Pass67

Pass68 hostile-reviewed and integrated corrected R&D Program7.

## Production changes

`kernel-durability` now distinguishes compact exact relation-data intents from full-target exact intents. Relation-data mutation codec v5 encodes the canonical delta once; metadata codec v4 persists `RelationDataExact` transaction identity. Older decoder paths remain readable.

`kernel-plan` adds `DerivedRelationTransitionRequest` and `DurableRuntime::commit_derived_relation_data`. The caller does not supply target Revision contents on this surface. The runtime verifies the named source is current, derives the target through the source-bound relation-update compiler, then enters the existing prepared/sealed durable publication boundary.

The legacy `commit_revision` path deliberately remains full-exact: because it accepts independent target Revision contents and `RevisionId` is nominal, it continues to persist the canonical full target witness.

## Hostile integration corrections/evidence

The original Pass63 same-ID/same-delta/different-target-content counterexample now remains on the legacy full-target surface and is rejected after checkpoint/compaction/reopen.

A new Pass68 hostile verifies:

- derived target == target constructed by the existing authoritative transition for the same source+delta;
- stale source requests for fresh transaction IDs are rejected before durable publication;
- reopen does not turn a rejected stale request into an idempotent success.

Normal prepared-transition sealing still rejects concurrent root/source drift before durable COMMIT.

## Scope boundary

Pass68 closes snapshot-sized relation-data intent duplication, not idempotency-history retention. `committed_transactions` is still copied into checkpoint metadata and has no production pruning/GC law. Streaming checkpoints, generic format migration, group commit, replication, MAC authentication and power-loss proof are unchanged.

## Verification

Full Rust 1.98.1 frozen-source gate PASS: fmt, check, debug tests, strict Clippy, release tests, release build, strict rustdoc and overflow-check release tests. Cold release/overflow compilation timeouts were not counted; warmed reruns passed.

Frozen snapshot: 425 tests, 167 kernel-plan tests, 21 crates, 59,817 Rust LOC, 0 unsafe, 19 pre-existing allow attributes, 0 TODO/FIXME/todo!/unimplemented!.
