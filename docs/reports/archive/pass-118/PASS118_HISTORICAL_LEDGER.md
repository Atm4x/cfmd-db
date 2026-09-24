# PASS118 HISTORICAL PROBLEMS LEDGER

Authoritative production status after Historical #17 external freshness / anti-rollback closeout.

## PROD CLOSED — 20 / 22

- #1 structural/custom semantic physical persistence and ordering — CLOSED Pass82.
- #2 PWRC positive recursive Bag execution — CLOSED Pass85.
- #3 unified physical lifecycle/capability/convergence — CLOSED Pass82.
- #4 autonomous telemetry/controller — CLOSED Pass83.
- #5 resource accounting / pressure separation — CLOSED Pass83.
- #6 Revision/Γ-bound OrderedView/pagination — CLOSED Pass90.
- #7 recovery rebuild economics + durable semantic-core rehydrate/WAL replay — CLOSED Pass84.
- #8 durable causal effect ledger / REIC branch lifecycle — CLOSED Pass92.
- #9 canonical durable-format migration registry — CLOSED Pass88.
- #10 semantic implementation package/auth/deployment — **PROD CLOSED Pass116**.
- #11 idempotency epochs / bounded exact retry history / payload GC — CLOSED Pass86.
- #12 streaming/chunked checkpoint + PreparedCutCapsule + exact shadow WAL — CLOSED Pass89.
- #14 authority-uncertainty restart / poison policy — CLOSED Pass87.
- #15 barrier-safe group commit / non-authoritative async batching — CLOSED Pass87.
- #16 replication/consensus runtime — **PROD CLOSED Pass115**.
- #17 authenticated durable store + external freshness/anti-rollback anchor — **PROD CLOSED Pass118**.
- #18 formal immutable-generation publication/rename/fsync/GC proof — **PROD CLOSED Pass112**.
- #19 bounded repair / VMF-OFC / verified observation transport — CLOSED Pass88.
- #21 maintained I64 Group constant-factor debt — PROD CLOSED Pass108.
- #22 maintained TopK constant-factor debt — PROD CLOSED Pass108.

## OPEN — 2 / 22

- #13 supported-platform real durability assurance — OPEN. Must validate the actual supported OS/filesystem semantics required by the #18 publication proof and #17 external-authority state publication.
- #20 formal surface-to-kernel mechanization — OPEN.

## #17 closure evidence

1. authenticated signed `FreshnessCut` and exact CAS identity — Pass117;
2. store generation/WAL binding and pre-recovery rollback/fork checks — Pass117;
3. automatic WAL/checkpoint external advancement and poison-on-ambiguity — Pass117;
4. real TCP process/provider authority with server-held signing key and separate persisted CAS state — Pass118;
5. provider-side monotonic fencing and exact-record CAS — Pass118;
6. independent-process restart/unavailable evidence — Pass118;
7. response-loss before/after apply convergence and full reopen hostile matrix — Pass118;
8. full workspace fmt/check/strict-Clippy/test gates — Pass118.

External freshness is fail-closed. No local fallback is permitted once a generation carries the external freshness binding.
