# PASS92 HISTORICAL LEDGER

Authoritative production status after Pass92.

## PROD CLOSED — 14 / 22
- #1 structural/custom semantic physical persistence and ordering — CLOSED Pass82.
- #2 PWRC positive recursive Bag execution — CLOSED Pass85.
- #3 unified physical lifecycle/capability/convergence — CLOSED Pass82.
- #4 autonomous telemetry/controller — CLOSED Pass83.
- #5 resource accounting / pressure separation — CLOSED Pass83.
- #6 Revision/Γ-bound OrderedView/pagination over supported native backends — CLOSED Pass90.
- #7 recovery rebuild economics + durable semantic-core rehydrate/WAL replay — CLOSED Pass84.
- #8 durable causal effect ledger / REIC DAG lifecycle — **CLOSED Pass92**. Local + non-head replicated effects now share one exact event ontology; branch heads, causal frontiers, replay, retirement and cross-local/remote ideals are durable.
- #9 canonical durable-format migration registry / canonical recovered-state boundary — CLOSED Pass88.
- #11 idempotency epochs / bounded exact retry history / payload GC — CLOSED Pass86.
- #12 streaming/chunked checkpoint + `PreparedCutCapsule` + exact shadow WAL — CLOSED Pass89.
- #14 authority-uncertainty restart classification / restart-poison policy — CLOSED Pass87.
- #15 barrier-safe group commit / non-authoritative async batching — CLOSED Pass87.
- #19 bounded repair / VMF-OFC verification / verified observation transport — CLOSED Pass88.

## PROD PARTIAL / ADVANCED
- #10 semantic implementation package / authentication / deployment — ADVANCED Pass91. Contract/artifact/refinement/authentication/runtime-authorization boundary is production and durable builtin reopen uses it. Remaining: external package/CAS, cryptographic signatures/trust roots/key rotation, sandbox/ABI, external proof formats and artifact distribution.
- #16 replication / consensus runtime — **ADVANCED Pass92**. Durable independent branch ingestion, ordered-slot uniqueness, origin namespaces, exact causal-cut checking, restart-safe branch lifecycle and monotone sequencer-epoch fencing are production. Remaining: membership, quorum certificates/voting, leader election, quorum failure handling, networking/anti-entropy and certified-confluent coordination-free admission.

## OPEN / externally or formally blocked
- #13 supported-platform durability profiles / destructive power-cut evidence — OPEN; needs real supported-platform evidence.
- #17 authenticated durable store + external freshness / anti-rollback anchor — OPEN; strict freshness needs non-rollback external authority.
- #18 formal immutable-generation publication / rename / fsync / GC proof — OPEN; requires mechanization under explicit filesystem axioms.
- #20 formal surface-to-kernel mechanization — OPEN.

## DEFERRED — external R&D ownership
- #21 I64 Group constant-factor lowering / benchmark closure — DEFERRED by user request; active external R&D branch, Pass92 production untouched.
- #22 TopK performance closure — DEFERRED by user request; active external R&D branch, Pass92 production untouched.

## Pass92 closure note — #8
#8 is closed because a durable branch no longer has to masquerade as the one published store head. Its exact causal events, branch head, causal frontier, restart reconstruction, retention/retirement and ideal are all first-class durable state while still reusing `DurableRevisionEffectRecord` / `DurableTransactionIntent`.

## Next production order
1. Continue #16 with durable membership epochs, quorum certificate semantics and explicit durability/visibility states; do not make network arrival semantic authority.
2. #10 only when actual external package/security execution substrate is implemented.
3. #13/#17/#18/#20 remain evidence/formal-bound; do not close them with mocks.
4. #21/#22 remain untouched until the external R&D package converges.
