# Pass120 — Historical #13 platform assurance hardening

## Status

- Base: Pass119 (`#20 PROD CLOSED`, 21/22 historical problems closed).
- #13 software/runtime side: **COMPLETE**.
- #13 historical status: **OPEN / CERTIFICATION-READY**.
- Historical ledger: **21 / 22 PROD CLOSED**.
- Source freeze: `2026-09-23T19:21:05Z`.

## Problem → hypothesis → implementation → falsification → result

### Problem

CFMD had formal publication proofs (#18), authenticated external freshness (#17), and extensive subprocess fault tests, but no production-supported-platform admission boundary. Nothing prevented an operator from running on an unsupported filesystem/device and assuming the #18 filesystem axioms held.

### Hypothesis

The software side of #13 can be completed by making platform assurance explicit and fail-closed: name the supported profiles, probe the live mount and fsync/rename primitives, require cryptographically authenticated evidence of a complete destructive power-loss campaign, and expose store entry points that cannot claim certified operation without that verified token.

### Implementation

See `PASS120_HIST13_PLATFORM_ASSURANCE.md`.

Production changes are confined to `kernel-durability`:

- new `platform_assurance` module;
- named Linux ext4/XFS profiles;
- mount/options parser and runtime fsync/rename/read-back probe;
- platform fingerprint;
- 7-cut destructive campaign preparation and post-reboot verifier;
- strict signed campaign certificate verification;
- certified create/open/external-freshness-open entry points;
- standalone `cfmd-durability-campaign` tool.

### Falsification

The actual sandbox mount was probed. It is overlayfs with `fsync=volatile`; both named profiles are rejected. The sandbox also lacks `CAP_SYS_ADMIN`, so a genuine loop/dm-flakey or hard-power campaign cannot be executed here. This prevents a false #13 closure.

Unit hostile tests cover unsafe mount options, wrong/non-block source, incomplete campaign, platform-fingerprint mismatch, signature tamper, torn manifest, and new authority without durable prerequisite.

### Result

The codebase is certification-ready, but #13 remains the sole OPEN historical problem until real destructive evidence is collected on a named supported device/filesystem profile.

## Gates

- `cargo fmt --all -- --check`: PASS.
- `cargo check --workspace --all-targets`: PASS.
- `cargo clippy --workspace --all-targets -- -D warnings`: PASS.
- `kernel-durability` platform-assurance focused tests: PASS.
- full workspace: **766 passed / 0 failed / 8 ignored = 774 declared**.
- current-sandbox real profile probe: correctly REJECTED.
