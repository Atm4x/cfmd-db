# PASS94 HISTORICAL LEDGER

Authoritative production status after Pass94.

## PROD CLOSED — 14 / 22
- #1 structural/custom semantic physical persistence and ordering — CLOSED Pass82.
- #2 PWRC positive recursive Bag execution — CLOSED Pass85.
- #3 unified physical lifecycle/capability/convergence — CLOSED Pass82.
- #4 autonomous telemetry/controller — CLOSED Pass83.
- #5 resource accounting / pressure separation — CLOSED Pass83.
- #6 Revision/Γ-bound OrderedView/pagination over supported native backends — CLOSED Pass90.
- #7 recovery rebuild economics + durable semantic-core rehydrate/WAL replay — CLOSED Pass84.
- #8 durable causal effect ledger / REIC DAG lifecycle — CLOSED Pass92.
- #9 canonical durable-format migration registry / canonical recovered-state boundary — CLOSED Pass88.
- #11 idempotency epochs / bounded exact retry history / payload GC — CLOSED Pass86.
- #12 streaming/chunked checkpoint + `PreparedCutCapsule` + exact shadow WAL — CLOSED Pass89.
- #14 authority-uncertainty restart classification / restart-poison policy — CLOSED Pass87.
- #15 barrier-safe group commit / non-authoritative async batching — CLOSED Pass87.
- #19 bounded repair / VMF-OFC verification / verified observation transport — CLOSED Pass88.

## PROD PARTIAL / ADVANCED
- #10 semantic implementation package / authentication / deployment — ADVANCED Pass91. Remaining: external package/CAS, cryptographic signature/trust-root/key-rotation stack, sandbox/ABI, external proof formats and artifact distribution.
- #16 replication / consensus runtime — **ADVANCED Pass94**. Durable branch authority, membership epochs, LocalDurable/QuorumDurable/Published stages, causal-prefix publication and restart-safe branch lifecycle were already production. Pass94 additionally makes effect quorum and membership reconfiguration depend on fsync-backed vote-once records. Remaining: authenticated vote evidence, election/locking/term protocol, quorum-loss/recovery, network/anti-entropy and multi-process distributed-fault assurance.

## V5 INTEGRATION IN PROGRESS
- #21 I64 Group constant-factor lowering / benchmark closure — NOT CLOSED. Pass94 integrates V5 Stage 1 universal Delta ABI only; Group execution and benchmark closure are unchanged.
- #22 TopK performance closure — NOT CLOSED. Same Stage-1 ABI substrate only; TopK execution and corrected benchmark closure are unchanged.

## OPEN / externally or formally blocked
- #13 supported-platform durability profiles / destructive power-cut evidence — OPEN; needs real supported-platform evidence.
- #17 authenticated durable store + external freshness / anti-rollback anchor — OPEN; strict freshness needs non-rollback external authority.
- #18 formal immutable-generation publication / rename / fsync / GC proof — OPEN; requires mechanization under explicit filesystem axioms.
- #20 formal surface-to-kernel mechanization — OPEN.

## Pass94 #16 safety note

A quorum acknowledgement set is no longer itself authority. Every acknowledgement used for effect certification must correspond to a durable vote for the same membership epoch, global decision position and effect. One voter cannot vote for two effects in that slot, even if proposals come from different leaders/sequencers. Membership successor votes are stronger still: one voter cannot bind to conflicting successors of the same previous membership epoch. Both rules survive restart because the votes are replayed from the replication journal.

This closes the Pass93 double-vote hole but not the entire replication/consensus row. Authentication and a real election/locking/network protocol remain required before #16 may be called CLOSED.

## V5 #21/#22 integration note

The supplied V5 R&D package is architecture-converged but explicitly production-pending. Pass94 follows its integration order and completes **Stage 1 only**: universal signed `DeltaView`/`DeltaSink`, compact common shapes, actual fixed inline storage with reusable spill, and a zero-copy `RelationDelta` adapter. No maintained operator has been switched to the ABI yet.

Next integration checkpoint: Stage 2 `ValidatedTransitionFrame`, retaining the already-computed source patch and certified effect without changing publication semantics.
