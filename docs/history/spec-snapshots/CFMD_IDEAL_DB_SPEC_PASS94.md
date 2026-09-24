# CFMD IDEAL DB SPEC — PASS94 ADDENDUM

This addendum extends the authoritative Pass93 specification.

## Durable consensus observations are not quorum authority until voted

A replication quorum certificate MUST be backed by durable vote evidence. Transport arrival, authenticated observation, local fsync and quorum authority are distinct stages.

For replicated effects, each voter has at most one durable vote at a given `(membership_epoch, global_decision_position)`. Leader/sequencer identity does not partition the decision slot. Therefore leader replacement cannot authorize the same voter to support a conflicting effect at the same ordered position.

For membership replacement, each voter may durably bind to at most one successor configuration of a previous membership epoch. A non-bootstrap membership change MUST carry a strict-majority acknowledgement set whose members already have matching durable votes for the exact successor and one common term.

These rules are intentionally safety-biased. They do not constitute a complete leader-election/locking protocol and may sacrifice liveness after an abandoned membership vote. A future consensus layer may weaken the conservative rule only with a proved safe lock/term protocol.

Authenticated peer identity remains an external security obligation. `ReplicaId` is an authority coordinate, not cryptographic evidence.

## Universal internal Delta ABI — V5 Stage 1

Public/query compatibility continues to use `RelationDelta`. Internally, CFMD now defines a representation-independent finite signed-effect interface:

`DeltaView<Row>` / `DeltaSink<Row>`.

The first production carrier implementations are:

- `CompactDelta` for common tiny effects;
- fixed `InlineDelta` storage;
- `AdaptiveDelta` with reusable spill storage;
- zero-copy `RelationDeltaView` over the legacy/public delta representation.

This stage changes no maintained-query execution semantics. It exists so subsequent integration can migrate internal edges one at a time while differential tests prove representation equivalence.

## Required next step for #21/#22

Validation/preparation should next produce `ValidatedTransitionFrame` objects that retain both the already-computed source mutation plan and the certified signed effect. Publication authority and candidate-state semantics MUST remain unchanged. Operator fusion and specialized Group/TopK lowerings must not precede this proof-producing preparation boundary.
