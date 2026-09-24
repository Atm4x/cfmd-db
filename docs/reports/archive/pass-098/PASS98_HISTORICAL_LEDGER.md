# PASS98 HISTORICAL LEDGER

Authoritative production status after Pass98.

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
- #21 maintained I64 Group constant-factor debt — V5 INTEGRATION IN PROGRESS: Stages 1–3, Stage 4 framework, 4.1 ZeroCrossing and 4.2 Annotation/Group complete; production performance closure not yet run.
- #22 maintained TopK constant-factor debt — V5 INTEGRATION IN PROGRESS: Stages 1–3 + common Stage 4 framework complete; **Stage 4.3a scalar-I64 physical lowering complete**, whole OrderedBoundary still partial because I64Rows/SemanticOrdered remain legacy.

## Pass98 closure note

Pass98 closes only the scalar-I64 physical substage of OrderedBoundary. The production scalar TopK path now has exact dense-unit/dense-counted/paged-radix tiering, fail-atomic promotion/fallback and universal signed planning. This is substantial #22 integration progress but not historical closure and not whole Stage 4.3 closure.

## Next

1. Pass99: finish Stage 4.3 by migrating `I64Rows` and `SemanticOrdered` to universal read-only plan/commit + `AdaptiveDelta` output.
2. Then Stage 4.4 Join and Stage 4.5 Blocker.
3. Stage 5: eliminate internal compatibility materialization and retain `RelationDelta` only at public/persistence boundaries.
4. Stage 6: corrected Group/TopK performance and allocation matrix before considering #21/#22 PROD CLOSED.
