# PASS86 HISTORICAL LEDGER

Authoritative baseline after Pass86.

## PROD CLOSED

- #1 structural/custom semantic physical persistence and ordering — CLOSED Pass82.
- #2 PWRC positive recursive Bag execution — CLOSED Pass85.
- #3 unified physical lifecycle/capability/convergence — CLOSED Pass82.
- #4 autonomous telemetry/controller — CLOSED Pass83.
- #5 resource accounting / pressure separation — CLOSED Pass83.
- #7 recovery rebuild economics + durable semantic-core rehydrate/WAL replay — CLOSED Pass84.
- #11 idempotency epochs / bounded exact retry history / payload GC — **CLOSED Pass86**.

Whole-row production closures: **7 / 22**. Remaining whole rows: **15 / 22**.

## #11 closure note

The portable design was strengthened during production rebase. Retry identity is `(epoch, tx-id)`, while Γ-REIC uses a separate WAL-persisted causal event identity. Causal records are self-contained, so retry payload GC cannot invalidate causal history. Crash-before-checkpoint after raw-id reuse is covered by hostile test.

## Still OPEN

#6 revision/Γ-bound pagination cursor + remaining layout parity; #8 durable causal effect ledger / REIC DAG lifecycle; #9 canonical durable-format migration registry; #10 semantic implementation package/auth/deployment; #12 streaming chunk checkpoint + PreparedCutCapsule + shadow WAL; #13 platform durability profiles / real fault assurance; #14 authority-uncertainty restart classification; #15 barrier-safe group commit; #16 replication/consensus runtime; #17 authenticated durable store + external freshness anchor; #18 formal publication/rename/fsync/GC proof; #19 bounded repair; #20 formal surface-to-kernel mechanization; #21 I64 Group constant-factor debt; #22 TopK performance closure gate.

## Next order

1. **#14** restart poison / authority-uncertainty classification.
2. **#15** barrier-safe group commit.
3. **#19** bounded repair / VMF/OFC candidate verification.

Do not re-open #11 by conflating retry-history GC with Γ-REIC causal-history retention; causal-history lifecycle remains #8.
