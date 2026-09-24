# PASS113 HISTORICAL PROBLEMS LEDGER

Authoritative production status after #16A term/election/locking integration.

## PROD CLOSED — 17 / 22

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
- #18 formal immutable-generation publication/rename/fsync/GC proof — PROD CLOSED Pass112.
- #19 bounded repair / VMF-OFC / verified observation transport — CLOSED Pass88.
- #21 maintained I64 Group constant-factor debt — PROD CLOSED Pass108.
- #22 maintained TopK constant-factor debt — PROD CLOSED Pass108.

## OPEN / PARTIAL — 5 / 22

- #10 semantic implementation package/auth/deployment — PROD PARTIAL (Pass91); external CAS/signature/trust-root/key-rotation/sandbox/ABI remain.
- #13 supported-platform real durability assurance — OPEN; platform validation boundary for #18 filesystem axioms.
- #16 replication/consensus runtime — **ADVANCED / PROD PARTIAL; 16A COMPLETE Pass113**. Durable term promises, vote-once leader election, stale-term fencing, quorum leader certificates, term-bound accepted values, durable per-position locks, explicit later-term carry-forward, joint old+new membership quorum and restart replay are now production code. Remaining: authenticated peer evidence, quorum-loss/recovery, transport/anti-entropy and distributed multi-process fault assurance.
- #17 authenticated durable store + external freshness/anti-rollback anchor — OPEN.
- #20 formal surface-to-kernel mechanization — OPEN.

## Pass111 engineering closeout

Typed compile-once metadata/direct-flat construction and attach-storage clone debt remain closed; detached revision-candidate shallow COW is intentional; Group hand-ceiling gap remains non-blocking evidence.

## Next frontier

Proceed to **#16B — authenticated peer evidence + quorum-loss/recovery**. Keep cryptographic trust-root/key lifecycle dependencies explicit relative to #10/#17. After 16B, implement transport/anti-entropy and a distributed multi-process fault matrix before considering #16 PROD CLOSED.

## Pass114 update — Historical #16

- #16A term/election/locking authority: COMPLETE (Pass113).
- #16B authenticated peer evidence + quorum-loss/recovery: **CLOSURE CANDIDATE**, implementation and package-level hostile tests complete, but full-workspace test gate was not completed before the Pass114 hard wall.
- #16 overall: **ADVANCED / PROD PARTIAL**.
- PROD CLOSED count remains **17/22**.
- Next: rerun the one missing workspace test gate first. If green, promote 16B COMPLETE, then proceed to 16C transport/anti-entropy/failure-detection/distributed multi-process fault matrix.
