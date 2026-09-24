# Pass120 — Historical #13 supported-platform durability assurance

## Status

- Historical #13: **OPEN / CERTIFICATION-READY**.
- Software/runtime assurance boundary: **COMPLETE Pass120**.
- Remaining blocker: **real destructive hard-power evidence on at least one named supported ext4/XFS device profile**.
- Historical ledger remains **21 / 22 PROD CLOSED**. #13 is the only OPEN problem.

## Problem

#18 proved CFMD's immutable-generation publication/GC protocol assuming the filesystem durability axioms used by `sync_all`, atomic rename and directory fsync. #17 separately protects against authenticated rollback. Until Pass120 the production API had no fail-closed way to establish that a store was actually running on a filesystem/device profile for which those axioms had been empirically validated.

The historical closure rule explicitly forbids calling process-kill tests or mocked filesystems sufficient evidence for #13.

## Implementation

Added `crates/kernel-durability/src/platform_assurance.rs`.

### Named supported profiles

Current certification vocabulary is intentionally narrow:

- `LinuxExt4Ordered`
- `LinuxXfs`

Admission is fail-closed. The runtime parser resolves the longest matching entry in `/proc/self/mountinfo` and rejects:

- another filesystem type;
- non-local-block-device mounts;
- `fsync=volatile`;
- `nobarrier`;
- ext4 `data=writeback`.

The live primitive probe performs the exact syscall class consumed by #18:

1. create/write file;
2. file `sync_all`;
3. directory `sync_all`;
4. atomic rename;
5. post-rename directory `sync_all`;
6. read-back;
7. remove + directory `sync_all`.

### Certified-store API

`DurableRevisionStore` now exposes explicit certified entry points:

- `create_on_supported_platform`
- `open_on_supported_platform`
- `open_with_external_freshness_on_supported_platform`

These do **not** accept a bare runtime probe. They require a `VerifiedDestructiveDurabilityCampaignEvidence` token.

The token can only be obtained by strict Ed25519 verification of `SignedDestructiveDurabilityCampaignEvidence` under a caller-supplied `TrustRootSet`. The canonical signed message binds:

- trust-root epoch;
- named profile;
- campaign id;
- exact runtime platform SHA-256 fingerprint;
- completed and expected case counts;
- campaign evidence SHA-256 digest.

All **7/7** required destructive cuts must be present. A zero evidence digest is rejected.

### Destructive hard-power harness

Added binary:

`cfmd-durability-campaign`

Commands:

- `probe <ext4|xfs> <directory>` — live supported-profile admission;
- `arm <ext4|xfs> <directory> <cut>` — prepare one exact filesystem cut and then stay alive indefinitely; an external controller must hard-power-cut the VM/machine while it remains ARMED;
- `verify <ext4|xfs> <directory> <cut>` — after reboot, accept only a crash image allowed by the #18 publication model.

Required cuts:

1. `after-candidate-file-sync`
2. `after-prerequisite-directory-sync`
3. `after-pending-manifest-sync`
4. `after-manifest-rename`
5. `after-manifest-directory-sync`
6. `after-obsolete-remove`
7. `after-obsolete-directory-sync`

The verifier specifically rejects torn manifest contents, new authority without its durable prerequisite, loss of old authority before publication, and loss of published authority during GC.

## Falsification / current real environment

The current execution sandbox was tested as a real platform rather than silently treated as supported.

Observed:

- Linux 6.18.44 x86_64;
- filesystem: `overlayfs`;
- mount source: `overlay`;
- mount option includes `fsync=volatile`;
- no `CAP_SYS_ADMIN` / `CAP_SYS_RAWIO`;
- no usable loop/device-mapper destructive setup.

Both `ext4` and `xfs` profile probes fail closed. See `PASS120_PLATFORM_CURRENT_ENV_EVIDENCE.txt`.

This is a useful negative assurance result: Pass120 will not certify the ChatGPT sandbox merely because `fsync(2)` returns success.

## Why #13 is still OPEN

Everything executable inside the repository is now in place, but the historical criterion requires empirical evidence that cannot be manufactured inside this sandbox: an actual hard-power/device-fault campaign on a named supported ext4/XFS storage profile.

A clean process exit, SIGKILL, subprocess crash, overlayfs test, or synthetic/mock filesystem remains explicitly insufficient.

To close #13, run all seven `arm -> hard power cut -> reboot -> verify` cases on the target profile, aggregate the logs into an evidence digest, sign the campaign certificate with the platform-assurance trust root, then verify that the resulting certificate is accepted by the certified-store API on the same platform fingerprint.
