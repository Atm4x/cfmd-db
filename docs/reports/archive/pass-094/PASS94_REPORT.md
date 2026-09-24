# PASS94 REPORT

**Status:** FINAL / CONSENSUS VOTE-ONCE SAFETY ADVANCE + HIST21/22 V5 STAGE-1 INTEGRATION

## Baseline and wall-clock boundary

Baseline: frozen Pass93 (`cfmd_workspace_pass93_membership_quorum_authority.zip`). Source work started at **2026-09-23 00:16:18 UTC**. Production source was frozen at **00:26:51 UTC**, well before the nominal 20-minute source cutoff. After freeze no `.rs` file or Cargo manifest was changed; only reports, manifest and packaging are produced.

The supplied `CFMD_RND_HIST21_22_2026-09-23_v5(1).zip` was audited before touching the maintained-query surface. V5 explicitly recommends staged production integration and does **not** claim #21/#22 production closure.

## Result

- Historical production closure remains **14 / 22**.
- Historical **#16 replication / consensus runtime — ADVANCED / PROD PARTIAL**.
- Historical **#21/#22 — V5 INTEGRATION STARTED, STAGE 1 ONLY**; neither row is closed.
- Production source delta versus Pass93 is exactly five files:
  - `crates/kernel-durability/src/lib.rs`
  - `crates/kernel-durability/src/replication.rs`
  - `crates/kernel-durability/src/store.rs`
  - `crates/kernel-query/src/lib.rs`
  - `crates/kernel-query/src/delta_abi.rs` (new)
- Frozen path-stable source fingerprint: `b28009479bcbfa8eb84ece69108f05c680d0f82d855b310d9e69d1dfd9386da8`.
- Final verification: fmt/check/strict Clippy/full workspace tests PASS; **685 declared tests / 0 failed / 8 ignored**.

## #16 — problem → hypothesis → implementation → falsification → result

### Problem

Pass93 had durable membership, quorum and publication stages, but its quorum certificate still trusted a structural acknowledgement set. Two conflicting majority certificates can intersect in one voter; without durable vote-once state that voter can support both decisions. Therefore threshold counting was not a consensus-safety proof.

### Hypothesis

The existing replication authority journal can close this local safety hole without changing the transaction WAL or creating a second mutation ontology if authenticated peer observations are converted into durable vote records **before** quorum certification.

### Implementation

Pass94 adds two durable journal frame classes:

1. `ReplicationEffectVote`
   - binds voter + membership epoch + exact REIC effect;
   - vote-once key is `(membership_epoch, global decision position, voter)`;
   - global decision position comes from the existing ordered effect envelope, so changing leader/sequencer identity does not create a new slot for the same position.

2. `ReplicationMembershipVote`
   - binds voter + previous membership epoch + term + exact successor membership;
   - one voter may bind to only one successor configuration of a previous epoch;
   - this is intentionally conservative: it prioritizes split-brain safety over liveness until a full election/locking protocol exists.

`ReplicationQuorumCertificate` is no longer sufficient as a bare set of replica IDs. Every member named by the certificate must have an exact durable effect vote for the same effect/slot. A non-bootstrap `ReplicationMembershipChange` likewise requires a consistent-term quorum of durable votes for its exact successor configuration.

Vote records are CRC-protected/fsync-backed frames in the existing replication authority journal and replay before later quorum/membership frames, so vote-once survives restart.

### Falsification

New hostiles verify:

- two different leaders/sequencers propose distinct effects at the same global decision position; one intersecting voter cannot durably vote for both;
- the conflicting second effect vote is still rejected after reopen;
- one voter cannot support two different successor memberships of the same previous epoch, even under a later term;
- the conflicting membership vote remains rejected after reopen;
- all previous quorum/publication/membership tests now build certificates from durable vote evidence rather than structural ID sets alone.

### Result

The specific Pass93 double-vote safety hole is closed in production. #16 nevertheless remains **PARTIAL**, because this is not yet a complete distributed consensus runtime. Still absent are authenticated vote bytes/trust-root verification, leader election / term advancement and locking, quorum-loss/recovery behavior, network transport/anti-entropy, and an end-to-end multi-process fault model. Numeric replica identity is not treated as authentication.

## #21/#22 V5 — Stage 1 integration only

### Problem

The converged V5 R&D architecture says internal maintained-query propagation should migrate from repeated owned `RelationDelta` edge materialization to one certified signed Delta ABI. It explicitly requires staged integration and forbids prematurely declaring #21/#22 closed.

### Stage-1 implementation

Pass94 introduces an execution-neutral carrier boundary in `kernel-query`:

- `DeltaView<R>` — representation-independent read interface over finite signed effects;
- `DeltaSink<R>` — representation-independent construction interface;
- `Weighted<R>`;
- `CompactDelta<R>` for Empty / One / Replace / Two common shapes;
- `InlineDelta<R, N>` with actual fixed inline storage;
- `AdaptiveDelta<R, N>` which spills only after inline capacity and reuses spill capacity after `clear`;
- `RelationDeltaView<'a>` — zero-copy compatibility adapter over current public `RelationDelta` removed/inserted vectors;
- `RelationDelta::as_delta_view()`.

No current maintained operator consumes this ABI yet. Group and TopK algorithms, validation ownership, maintained-tree propagation, benchmark paths and public `RelationDelta` behavior are unchanged.

### Stage-1 hostile verification

- compact replacement visits exactly `(-1, removed), (+1, inserted)`;
- adaptive spill capacity is reused after clearing;
- `RelationDeltaView` preserves exact row addresses, proving the compatibility adapter is zero-copy rather than a hidden clone/materialization.

## Deliberate non-claims

Pass94 does **not** claim:

- #16 consensus/network closure;
- authenticated cryptographic votes;
- leader election or Raft/Paxos-equivalent liveness/safety protocol;
- #21 Group constant-factor closure;
- #22 TopK closure;
- `ValidatedTransitionFrame` integration;
- linear-island fusion;
- barrier-kernel migration;
- corrected Group/TopK production benchmark closure.

## Next

Pass95 should continue the V5 integration sequence at **Stage 2 — proof-producing leaf preparation / `ValidatedTransitionFrame`**, while preserving the existing candidate-state clone/publication semantics. Only after that should linear-island compilation and stateful barrier migration begin.

For #16, the next honest systems step is authenticated voter evidence + explicit election/locking/quorum-loss protocol. It should not be conflated with the #21/#22 execution migration.
