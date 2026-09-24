# PASS93 HISTORICAL LEDGER

Authoritative production status after Pass93.

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
- #16 replication / consensus runtime — **ADVANCED Pass93**. Pass92 branch authority plus Pass93 durable membership epochs, previous-membership reconfiguration quorum, explicit LocalDurable/QuorumDurable/Published stages, quorum certificates, causal-prefix publication and restart-safe published branch heads are production. Remaining blocker is actual consensus safety: authenticated durable vote-once records, election/decision-slot protocol, cross-replica safe reconfiguration, quorum-loss handling, networking/anti-entropy and certified-confluent coordination-free admission.

## OPEN / externally or formally blocked
- #13 supported-platform durability profiles / destructive power-cut evidence — OPEN; needs real supported-platform evidence.
- #17 authenticated durable store + external freshness / anti-rollback anchor — OPEN; strict freshness needs non-rollback external authority.
- #18 formal immutable-generation publication / rename / fsync / GC proof — OPEN; requires mechanization under explicit filesystem axioms.
- #20 formal surface-to-kernel mechanization — OPEN.

## DEFERRED — external R&D ownership
- #21 I64 Group constant-factor lowering / benchmark closure — DEFERRED by user request. Latest supplied v4 additionally explores a universal `DeltaView` / `ValidatedTransitionFrame` ABI, generic compact replacement, reusable spill arena and fused linear delta islands. Pass93 production does not touch this surface.
- #22 TopK performance closure — DEFERRED by user request; same external R&D ownership, production untouched.

## Pass93 #16 blocker note

Structural majority counting is not yet a consensus proof. Two distinct old-majority certificates always intersect, but without durable authenticated vote-once state the intersecting voter may double-vote for incompatible next configurations. Pass94 must address this protocol-level safety gap before network transport is treated as authoritative.
