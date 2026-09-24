# PASS97 HISTORICAL LEDGER

Authoritative production status after Pass97.

## PROD CLOSED — 14 / 22

- #1 structural/custom semantic physical persistence and ordering — CLOSED Pass82.
- #2 PWRC positive recursive Bag execution — CLOSED Pass85.
- #3 unified physical lifecycle/capability/convergence — CLOSED Pass82.
- #4 autonomous telemetry/controller — CLOSED Pass83.
- #5 resource accounting / pressure separation — CLOSED Pass83.
- #6 Revision/Γ-bound OrderedView/pagination — CLOSED Pass90.
- #7 recovery rebuild economics + durable semantic-core rehydrate/WAL replay — CLOSED Pass84.
- #8 durable causal effect ledger / REIC branch lifecycle — CLOSED Pass92.
- #9 canonical durable-format migration registry — CLOSED Pass88.
- #11 idempotency epochs / bounded exact retry history / payload GC — CLOSED Pass86.
- #12 streaming/chunked checkpoint + PreparedCutCapsule + exact shadow WAL — CLOSED Pass89.
- #14 authority-uncertainty restart / poison policy — CLOSED Pass87.
- #15 barrier-safe group commit / non-authoritative async batching — CLOSED Pass87.
- #19 bounded repair / VMF-OFC / verified observation transport — CLOSED Pass88.

## OPEN / PARTIAL — 8 / 22

- #10 semantic implementation package/auth/deployment — PROD PARTIAL (Pass91); external CAS/signature/trust-root/sandbox/ABI remain.
- #13 supported-platform real durability assurance — OPEN.
- #16 replication/consensus runtime — ADVANCED / PROD PARTIAL (Pass94); durable vote-once safety exists, but authenticated peer evidence, election/term protocol, quorum-loss recovery and transport/anti-entropy remain.
- #17 authenticated durable store + external freshness/anti-rollback anchor — OPEN.
- #18 formal immutable-generation publication/fsync/GC proof — OPEN.
- #20 formal surface-to-kernel mechanization — OPEN.
- #21 maintained I64 Group constant-factor debt — V5 INTEGRATION IN PROGRESS: Stages 1–3, Stage 4 framework, 4.1 ZeroCrossing and **4.2 Annotation/Group complete**; production closure gate not yet run.
- #22 maintained TopK constant-factor debt — V5 INTEGRATION IN PROGRESS: Stages 1–3 + common Stage 4 framework complete; 4.3 OrderedBoundary/TopK not started.

## Pass97 closure note

Stage 4.2 is an integration-stage closure, not historical #21 closure. Group now crosses the certified ABI and includes the v3 dense-window exact-I64 physical lowering with fallback and exact-count move microkernel. #21 remains open until the remaining v5 stages and production benchmark/closure matrix are completed.

## Next

1. Pass98: Stage 4.3 OrderedBoundary/TopK, including v3 DenseUnit/DenseCounted/PagedRadix promotion/fallback and universal plan/commit.
2. Then Stage 4.4 Join, Stage 4.5 Blocker, Stage 5 root-only materialization.
3. Run v5 Stage 6 corrected performance/allocation closure before considering #21/#22 PROD CLOSED.
