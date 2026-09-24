# PASS112 HISTORICAL PROBLEMS LEDGER

Authoritative production status after mechanized closure of historical #18.

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
- #18 formal immutable-generation publication/rename/fsync/GC proof — **PROD CLOSED Pass112**. Lean 4.34.0 theorem artifact proves P18.1–P18.10; source-binding gate covers ordinary + streaming publication and GC; Rust finite model and subprocess-kill matrices independently pass. Filesystem/platform assumptions remain #13.
- #19 bounded repair / VMF-OFC / verified observation transport — CLOSED Pass88.
- #21 maintained I64 Group constant-factor debt — PROD CLOSED Pass108.
- #22 maintained TopK constant-factor debt — PROD CLOSED Pass108.

## OPEN / PARTIAL — 5 / 22

- #10 semantic implementation package/auth/deployment — PROD PARTIAL (Pass91); external CAS/signature/trust-root/key-rotation/sandbox/ABI remain.
- #13 supported-platform real durability assurance — OPEN; requires real supported-platform destructive/fault evidence and is the platform validation boundary for #18 filesystem axioms.
- #16 replication/consensus runtime — ADVANCED / PROD PARTIAL (Pass94); next active frontier: term/election/locking, authenticated peer evidence, quorum-loss/recovery, transport/anti-entropy and distributed multi-process fault assurance.
- #17 authenticated durable store + external freshness/anti-rollback anchor — OPEN.
- #20 formal surface-to-kernel mechanization — OPEN.

## Pass111 engineering closeout

The five post-Stage-6 engineering items remain resolved/classified exactly as in Pass111: typed compile-once metadata and direct-flat construction are closed; attach-storage whole-state clone is closed; detached revision-candidate shallow COW clone is intentional; Group hand-ceiling gap is evidence, not an open blocker.

## Next frontier

Proceed to **#16A — first-class term/election/locking authority** on top of the now-mechanized durable publication substrate. Do not conflate #16 cryptographic trust/root work with #10/#17; dependencies must stay explicit.
