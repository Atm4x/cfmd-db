# PASS96 HISTORICAL LEDGER

Authoritative production status after Pass96.

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
- #16 replication / consensus runtime — ADVANCED Pass94. Durable branch authority, membership/quorum/publication lifecycle and fsync-backed vote-once safety are production. Remaining: authenticated vote evidence, election/locking/term protocol, quorum-loss/recovery, network/anti-entropy and multi-process distributed-fault assurance.

## V5 #21/#22 INTEGRATION IN PROGRESS
- #21 I64 Group constant-factor lowering / benchmark closure — NOT CLOSED. **Stages 1–3 complete; Stage 4 framework + 4.1 ZeroCrossing complete.** Annotation/Group remains next and must include the v3 dense-window lowering plus production differential/benchmark gates.
- #22 TopK performance closure — NOT CLOSED. **Stages 1–3 complete; Stage 4 framework + 4.1 ZeroCrossing complete.** OrderedBoundary/TopK remains untouched until Annotation/Group is closed.

## OPEN / externally or formally blocked
- #13 supported-platform durability profiles / destructive power-cut evidence — OPEN.
- #17 authenticated durable store + external freshness / anti-rollback anchor — OPEN.
- #18 formal immutable-generation publication / rename / fsync / GC proof — OPEN.
- #20 formal surface-to-kernel mechanization — OPEN.

## Pass96 V5 integration note

Stage 4 now has an explicit physical barrier map rather than an implicit “whatever is not a linear island” boundary. The map is exactly the current DTC partition: ZeroCrossing, Annotation, OrderedBoundary, BilinearPullback and BlockerZeroCrossing.

Stage 4.1 is the first migrated barrier: support normalization consumes the universal signed Delta ABI and uses plan-before-commit patches. Failed planning cannot mutate authoritative maintained state.

Stage 4.2 is deliberately untouched because v5 requires the real v3 Group physical lowering, not only a type-level kernel wrapper. Next order remains Annotation/Group -> OrderedBoundary/TopK -> BilinearPullback/Join -> BlockerZeroCrossing, then Stage 5 root-only compatibility materialization and Stage 6 production benchmark closure.
