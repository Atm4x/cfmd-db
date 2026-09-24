# CFMD IDEAL DB SPEC — PASS92 ADDENDUM

This addendum is normative over the Pass91 specification for durable causal replication authority.

## Durable causal authority beyond one publication head
A CFMD store may retain multiple non-published causal branches. A branch is not a second `Revision` publication authority and does not weaken the linear transaction WAL. Its authority is the same exact REIC event ontology used by local commits:

`DurableRevisionEffectRecord = (effect_id, prerequisites, exact intent, source revision, target revision, retry identity)`.

Local and replicated event identity namespaces MUST be disjoint. Local events occupy origin namespace zero. Replicated event identity is `(ReplicaId, origin_sequence)` encoded injectively in `RevisionEffectId`.

## Branch journal
Replicated/non-head effects MUST be persisted in a durable append-only journal separate from the linear transaction WAL. Admission of an effect MUST NOT advance `durable_head` or make it visible to readers.

For every admitted effect:
1. the exact effect identity MUST match its origin namespace and sequence;
2. the exact intent MUST be executable under available semantic implementation packages;
3. its causal prerequisites MUST equal the authoritative frontier of its source revision, or the union of exact parent frontiers for a multi-parent resolution;
4. every remote prerequisite MUST already be durable (down-closure);
5. the target revision MUST not already belong to another local or replicated frontier;
6. its branch MUST either be new at a known source frontier or advance the current non-retired head of the same branch.

The journal MUST checksum complete frames, tolerate only a torn incomplete final frame, fsync before returning durable admission, and replay/validate branch state on reopen.

## Ordered admission for current effects
Until an effect carries a durable confluence/coherence certificate, it is `OpaqueNonConfluent` and MUST NOT be admitted coordination-free. A remote effect therefore requires an externally established `DurableSequencerOrder = (sequencer, epoch, position)`.

The durable store MUST enforce:
- one effect per exact ordered slot;
- monotone sequencer epoch fencing;
- stale-epoch rejection;
- idempotent duplicate delivery of the same exact effect.

The ordering witness is authority input, not evidence that this crate itself ran consensus.

## Branch retirement
Retirement is a durable lifecycle transition bound to the expected branch head. A retired branch identity MUST NOT be advanced again after restart. Effect history remains readable so causal ideals are not torn by branch retirement.

## Replica ideal
The causal ideal of a replicated branch is reconstructed over the union of local and replicated event records. It MUST be down-closed and independent of delivery order.

## Replication durability stages
The following stages remain semantically distinct:
1. received in memory;
2. local durable branch admission;
3. quorum durable / consensus-authorized;
4. published to readers.

Pass92 implements stage 2 plus ordered-witness/fencing validation. It does not collapse stages 2–4.

## Explicit non-claims
Pass92 does not implement cluster membership, quorum vote collection/certificates, leader election, network transport, quorum-loss policy, signatures on remote ordering witnesses, or coordination-free certified-confluent anti-entropy. Those are required before historical #16 can be closed.
