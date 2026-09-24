# PASS95 HISTORICAL LEDGER

Authoritative production status after Pass95.

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
- #21 I64 Group constant-factor lowering / benchmark closure — NOT CLOSED. **Stages 1–3 complete**. Stage 1 universal signed Delta ABI landed Pass94; Pass95 adds proof-producing leaf frames and compiled maximal linear islands. Group barrier/lowering and benchmark closure are not yet ported.
- #22 TopK performance closure — NOT CLOSED. **Stages 1–3 complete**. TopK remains behind the existing ordered-boundary implementation until Stage 4 ports the barrier kernel and later stages apply the v5/v3 physical lowering and corrected production benchmark gate.

## OPEN / externally or formally blocked
- #13 supported-platform durability profiles / destructive power-cut evidence — OPEN.
- #17 authenticated durable store + external freshness / anti-rollback anchor — OPEN.
- #18 formal immutable-generation publication / rename / fsync / GC proof — OPEN.
- #20 formal surface-to-kernel mechanization — OPEN.

## Pass95 V5 integration note

Stage 2 removes the concrete duplicated source mutation planning identified by the v5 R&D integration map. Validation now produces durable-in-memory proof objects for the exact compiled Scan edge and commit consumes those objects. Same-relation multi-leaf plans are edge-disjoint.

Stage 3 derives `CompiledDeltaProgram` from the existing exact `RelDifferentialProgram`. Maximal `Linear` chains compile into `LinearIslandNormalForm`; predicates use source coordinates and intermediate Bag projections disappear. Stateful/zero-crossing classes remain explicit barriers.

Stage 4 is intentionally untouched in Pass95. Next order from v5 remains: ZeroCrossing -> Annotation/Group -> OrderedBoundary/TopK -> BilinearPullback/Join -> BlockerZeroCrossing, each under differential tests before proceeding.
