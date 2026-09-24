# Historical Problems Ledger

Status after Pass120 destructive certification and Pass121 repository hardening.

## PROD CLOSED — 22 / 22

- #1 structural/custom semantic physical persistence and ordering — CLOSED Pass82.
- #2 PWRC positive recursive Bag execution — CLOSED Pass85.
- #3 unified physical lifecycle/capability/convergence — CLOSED Pass82.
- #4 autonomous telemetry/controller — CLOSED Pass83.
- #5 resource accounting / pressure separation — CLOSED Pass83.
- #6 Revision/Γ-bound OrderedView/pagination — CLOSED Pass90.
- #7 recovery rebuild economics + durable semantic-core rehydrate/WAL replay — CLOSED Pass84.
- #8 durable causal effect ledger / REIC branch lifecycle — CLOSED Pass92.
- #9 canonical durable-format migration registry — CLOSED Pass88.
- #10 semantic implementation package/auth/deployment — PROD CLOSED Pass116.
- #11 idempotency epochs / bounded exact retry history / payload GC — CLOSED Pass86.
- #12 streaming/chunked checkpoint + PreparedCutCapsule + exact shadow WAL — CLOSED Pass89.
- #13 supported-platform real durability assurance — PROD CLOSED after Pass120 certification for the declared QEMU/TCG + Linux/ext4 profile.
- #14 authority-uncertainty restart / poison policy — CLOSED Pass87.
- #15 barrier-safe group commit / non-authoritative async batching — CLOSED Pass87.
- #16 replication/consensus runtime — PROD CLOSED Pass115.
- #17 authenticated durable store + external freshness/anti-rollback anchor — PROD CLOSED Pass118.
- #18 formal immutable-generation publication/rename/fsync/GC proof — PROD CLOSED Pass112.
- #19 bounded repair / VMF-OFC / verified observation transport — CLOSED Pass88.
- #20 formal surface-to-kernel mechanization — PROD CLOSED Pass119.
- #21 maintained I64 Group constant-factor debt — PROD CLOSED Pass108.
- #22 maintained TopK constant-factor debt — PROD CLOSED Pass108.

## #13 certification scope

The closure evidence certifies this explicit profile:

- QEMU 8.2.2, TCG software acceleration;
- Alpine Linux 3.24.2;
- Linux 6.18.52-0-virt x86_64;
- dedicated 512 MiB raw virtio data device;
- ext4 `data=ordered`;
- QEMU device cache configuration `cache=none,aio=threads`;
- seven required destructive cuts, each killing the whole QEMU process after the exact ARMED marker and verifying after a fresh boot;
- signed campaign/token verification and certified store create/open.

A bare-metal NVMe/SATA profile, XFS profile, different kernel, different QEMU/device/cache configuration or other storage stack requires its own certification campaign. That extends the support matrix; it does not reopen the historical architecture problem.

Evidence: `artifacts/certification/pass120-qemu-ext4/`.
