# PASS109 HISTORICAL PROBLEMS LEDGER

Authoritative production status after Flat NodeId Arena completion.

## PROD CLOSED — 16 / 22

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
- #21 maintained I64 Group constant-factor debt — PROD CLOSED Pass108 after V5 Stages 1–6 and corrected whole-chain/allocation/differential gates.
- #22 maintained TopK constant-factor debt — PROD CLOSED Pass108 after V5 Stages 1–6 and corrected whole-chain/allocation/differential gates.

## OPEN / PARTIAL — 6 / 22

- #10 semantic implementation package/auth/deployment — PROD PARTIAL (Pass91); external CAS/signature/trust-root/key-rotation/sandbox/ABI remain.
- #13 supported-platform real durability assurance — OPEN; requires real supported-platform destructive/fault evidence.
- #16 replication/consensus runtime — ADVANCED / PROD PARTIAL (Pass94); durable branch authority, membership epochs, vote-once records, quorum/publication stages exist. Remaining: authenticated peer evidence, election/locking/term protocol, quorum-loss/recovery, transport/anti-entropy and multi-process distributed-fault assurance.
- #17 authenticated durable store + external freshness/anti-rollback anchor — OPEN.
- #18 formal immutable-generation publication/rename/fsync/GC proof — OPEN; requires mechanization under explicit filesystem axioms and a mapping from production store events to model transitions.
- #20 formal surface-to-kernel mechanization — OPEN.

## Pass109 engineering closeout

Flat NodeId Arena is COMPLETE but does not change the 16/22 historic count: it was a post-#21/#22 engineering cleanup. Release runtime now has one maintained-state ownership layout aligned with ExecGraph V4 NodeIds.

## Next frontier

1. **#18 first**: formalize the exact immutable-generation publication + crash + GC state machine and prove authority/recoverability invariants against explicit filesystem axioms. This gives a crisp durable substrate before extending distributed authority.
2. **#16 second**: build the missing election/term/locking and authenticated quorum evidence on the existing durable vote-once journal, then anti-entropy/quorum-loss/multi-process fault assurance.
3. Do not relabel local mocks as closure for #13/#17/#18 or network simulation alone as closure for #16.
