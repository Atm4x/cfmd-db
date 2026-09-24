# Historical Problems Ledger — Pass120

## PROD CLOSED — 21 / 22

- #1 structural/custom semantic physical persistence & ordering — CLOSED Pass82.
- #2 PWRC positive recursive Bag execution — CLOSED Pass85.
- #3 unified physical lifecycle/capability/convergence — CLOSED Pass82.
- #4 autonomous telemetry/controller — CLOSED Pass83.
- #5 resource accounting / pressure separation — CLOSED Pass83.
- #6 Revision/Γ-bound OrderedView/pagination — CLOSED Pass90.
- #7 recovery rebuild economics + durable semantic-core rehydrate/WAL replay — CLOSED Pass84.
- #8 durable causal effect ledger / REIC branch lifecycle — CLOSED Pass92.
- #9 canonical durable-format migration registry — CLOSED Pass88.
- #10 semantic implementation package/auth/deployment — CLOSED Pass116.
- #11 idempotency epochs / bounded exact retry history / payload GC — CLOSED Pass86.
- #12 streaming/chunked checkpoint + PreparedCutCapsule + exact shadow WAL — CLOSED Pass89.
- #14 authority-uncertainty restart / poison policy — CLOSED Pass87.
- #15 barrier-safe group commit / non-authoritative async batching — CLOSED Pass87.
- #16 replication/consensus runtime — CLOSED Pass115.
- #17 authenticated durable store + external freshness/anti-rollback anchor — CLOSED Pass118.
- #18 formal immutable-generation publication/fsync/GC proof — CLOSED Pass112.
- #19 bounded repair / VMF-OFC / verified observation transport — CLOSED Pass88.
- #20 formal surface-to-kernel mechanization — CLOSED Pass119.
- #21 maintained I64 Group constant-factor debt — CLOSED Pass108.
- #22 maintained TopK constant-factor debt — CLOSED Pass108.

## OPEN — 1 / 22

### #13 supported-platform real durability assurance — OPEN / CERTIFICATION-READY Pass120

Software side completed in Pass120:

1. named supported profiles `LinuxExt4Ordered` and `LinuxXfs`;
2. fail-closed `/proc/self/mountinfo` profile admission;
3. live file-fsync / directory-fsync / rename / read-back probe;
4. exact platform SHA-256 fingerprint;
5. seven-cut destructive power-loss campaign harness matching #18 filesystem axioms;
6. post-reboot crash-image verifier;
7. signed Ed25519 campaign certificate, trust-root epoch bound;
8. certified store create/open APIs require a verified destructive campaign token;
9. current overlayfs `fsync=volatile` sandbox is explicitly rejected.

Remaining closure blocker: execute all seven cases with an actual hard power cut or equivalent device-level fault on a named supported ext4/XFS device profile and retain/sign the empirical evidence bundle. Process kill, mock filesystems, or this overlayfs sandbox do not satisfy the historical closure criterion.
