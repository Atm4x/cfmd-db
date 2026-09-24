# PASS91 HISTORICAL LEDGER

Authoritative production status after Pass91.

## PROD CLOSED — 13 / 22
- #1 structural/custom semantic physical persistence and ordering — CLOSED Pass82.
- #2 PWRC positive recursive Bag execution — CLOSED Pass85.
- #3 unified physical lifecycle/capability/convergence — CLOSED Pass82.
- #4 autonomous telemetry/controller — CLOSED Pass83.
- #5 resource accounting / pressure separation — CLOSED Pass83.
- #6 Revision/Γ-bound OrderedView/pagination over supported native backends — CLOSED Pass90.
- #7 recovery rebuild economics + durable semantic-core rehydrate/WAL replay — CLOSED Pass84.
- #9 canonical durable-format migration registry / canonical recovered-state boundary — CLOSED Pass88.
- #11 idempotency epochs / bounded exact retry history / payload GC — CLOSED Pass86.
- #12 streaming/chunked checkpoint + `PreparedCutCapsule` + exact shadow WAL — CLOSED Pass89.
- #14 authority-uncertainty restart classification / restart-poison policy — CLOSED Pass87.
- #15 barrier-safe group commit / non-authoritative async batching — CLOSED Pass87.
- #19 bounded repair / VMF-OFC verification / verified observation transport — CLOSED Pass88.

## PROD PARTIAL / ADVANCED
- #8 durable causal effect ledger / REIC DAG lifecycle — **ADVANCED Pass91**. Restart-safe effect identities, causal ideals/frontiers and exact multi-parent cuts already exist; Pass91 adds explicit durable effect kind and conservative `OpaqueNonConfluent` classification. Remaining: independent durable branch-head ingestion/retention, now coupled to #16.
- #10 semantic implementation package / authentication / deployment — **ADVANCED Pass91**. Contract/artifact/refinement/authentication/runtime-authorization boundary is production and durable builtin reopen uses it. Remaining: external package/CAS, real signature/trust-root/key rotation, sandbox/ABI, external proofs and artifact distribution.

## OPEN / externally blocked
- #13 supported-platform durability profiles / destructive power-cut evidence — OPEN; needs real supported-platform evidence.
- #16 replication / consensus runtime — OPEN; next coherent implementation wave with #8.
- #17 authenticated durable store + external freshness / anti-rollback anchor — OPEN; strict freshness needs non-rollback external authority.
- #18 formal immutable-generation publication / rename / fsync / GC proof — OPEN; requires mechanization under explicit filesystem axioms.
- #20 formal surface-to-kernel mechanization — OPEN.

## DEFERRED — external R&D ownership
- #21 I64 Group constant-factor lowering / benchmark closure — **DEFERRED by user request**. Active external R&D branch; Pass91 production untouched.
- #22 TopK performance closure — **DEFERRED by user request**. Active external R&D branch; Pass91 production untouched.

## Next production order
1. #8/#16 as one conservative replication-authority wave: exact durable effect envelopes, independent branch-head lifecycle and ordered admission for `OpaqueNonConfluent` effects. Do not create a second mutation ontology.
2. #10 only after the missing external execution-security substrate is actually available; do not relabel descriptor identity as binary cryptographic authentication.
3. #13/#17/#18 remain evidence/formal-bound and must not be closed with local mocks.
4. #21/#22 remain untouched until the user returns the converged R&D package for integration.
