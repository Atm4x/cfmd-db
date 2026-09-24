# PASS115 HISTORICAL PROBLEMS LEDGER

Authoritative production status after Historical #16 consensus runtime closeout.

## PROD CLOSED — 18 / 22

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
- #16 replication/consensus runtime — **PROD CLOSED Pass115**.
  - 16A Pass113: durable term promises, vote-once leader election, stale-term fencing,
    quorum leader certificates, term-bound accepted values, per-position decision locks,
    later-term carry-forward and joint old+new membership quorum.
  - 16B Pass114/115: current-epoch Ed25519 peer evidence, durable trust policy,
    quorum-loss fencing, authenticated recovery quorum and lock-frontier reconciliation.
  - 16C Pass115: canonical authenticated transport frames, store routing of authority
    evidence, bounded anti-entropy summary/chunks, advisory failure detection that can
    only fence authority, and a real multi-process tamper/torn/replay fault matrix.
- #18 formal immutable-generation publication/rename/fsync/GC proof — PROD CLOSED Pass112.
- #19 bounded repair / VMF-OFC / verified observation transport — CLOSED Pass88.
- #21 maintained I64 Group constant-factor debt — PROD CLOSED Pass108.
- #22 maintained TopK constant-factor debt — PROD CLOSED Pass108.

## OPEN / PARTIAL — 4 / 22

- #10 semantic implementation package/auth/deployment — **PROD CLOSED Pass116**; `kernel-auth` provides SHA-256/Ed25519 strict verification, trust-root rotation and authenticated artifact/CAS authority; `kernel-deployment` now provides canonical package/policy/ABI, filesystem package/CAS adapters, exact pinned-Γ descriptor/refinement authorization, and a Linux out-of-process namespace sandbox backend with network isolation and bounded execution.
- #13 supported-platform real durability assurance — OPEN; platform validation boundary for #18 filesystem axioms.
- #17 authenticated durable store + external freshness/anti-rollback anchor — OPEN.
- #20 formal surface-to-kernel mechanization — OPEN.

## #16 closure rationale

The original remaining #16 obligations from the historical ledger are all represented by production code and falsification evidence:

1. term/election/locking authority — Pass113;
2. authenticated peer evidence — Pass114;
3. quorum-loss/recovery — Pass114;
4. authenticated transport + anti-entropy — Pass115;
5. failure detection that cannot manufacture authority — Pass115;
6. distributed multi-process fault assurance — Pass115.

Failure detection remains deliberately advisory: false suspicion can reduce liveness by
installing the durable quorum-loss fence, but recovery still requires an authenticated
membership quorum. Transport session sequence replay protection is not durable authority;
restart safety continues to come from the durable vote/lock/authentication journal.

## Next historical frontier

Remaining historical problems: #13, #17 and #20. The most direct durability/security
continuation is #17, with #13 needed to discharge the real-platform assumptions used by #18.

## Pass117 update

- #17 remains OPEN / PROD PARTIAL.
- Added signed external freshness cuts, generation predecessor binding in metadata v13, anchored open/adoption, automatic WAL/checkpoint CAS advancement, and bypass prevention.
- Remaining closure work: real separate rollback-domain backend + independent-process hostile matrix + full workspace gates.
- PROD CLOSED count remains 19/22.
