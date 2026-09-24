# PASS120 Historic #13 — Final Certification Closeout

## Result

**Historic #13: PROD CLOSED for the explicitly certified virtualized durability profile below.**

This closeout does **not** claim bare-metal NVMe/SATA controller power-loss behavior. The certified support claim is scoped to the named QEMU/TCG + Linux/ext4 stack exercised here.

## Certified profile

- QEMU: 8.2.2, TCG software acceleration
- guest: Alpine Linux 3.24.2
- guest kernel: Linux 6.18.52-0-virt x86_64
- data device: dedicated 512 MiB raw virtio block device
- QEMU data-device configuration: `ile=raw,if=virtio,cache=none,aio=threads`
- filesystem: ext4
- required filesystem mode: `data=ordered`
- observed mount source: `/dev/vda`
- network: disabled
- Pass120 campaign binary: release build with Rust 1.98.1, built offline

Runtime platform fingerprint produced by Pass120:

`e96b6014187ffdece8bb6b310456a533d3fad39f1b86a7987441fa4b6d25db92`

## Destructive hard-power matrix

Authoritative matrix: **7 / 7 PASS**.

For every required cut:

1. fresh guest booted over the persistent ext4 image;
2. official Pass120 campaign re-admitted the exact `LinuxExt4Ordered` profile;
3. controller waited for the exact `ARMED cut=...` marker;
4. host sent SIGKILL to the **entire QEMU process**, not merely to the CFMD process;
5. a new QEMU process booted over the same virtual block device;
6. ext4 performed journal recovery where required;
7. official verifier emitted `DESTRUCTIVE_CASE_PASS`.

Passed cuts:

- `after-candidate-file-sync`
- `after-prerequisite-directory-sync`
- `after-pending-manifest-sync`
- `after-manifest-rename`
- `after-manifest-directory-sync`
- `after-obsolete-remove`
- `after-obsolete-directory-sync`

Aggregated evidence digest:

`e651425e80bc4d837544109637944a0e85c265c197dbc568d089fc180980b7d5`

## Signed campaign/token acceptance

A fresh guest boot on the same persistent device executed the Pass120 `cfmd-hist13-certify` path.

Observed production-like results:

- live supported-profile verification: **PASS**
- platform fingerprint equality: **PASS**
- Ed25519 campaign signature verification: **PASS**
- verified destructive campaign token construction: **PASS**
- `DurableRevisionStore::create_on_supported_platform`: **PASS**
- `DurableRevisionStore::open_on_supported_platform`: **PASS**

Guest output contained:

`CERTIFICATE_VERIFY=PASS`

`CERTIFIED_STORE_CREATE_OPEN=PASS`

`CFMD_CERTIFY_RC=0`

Signer key id:

`9301d95bd0e723c1eb8d19eba4229554e941d6e4997724563fac568095c829d9`

Public key:

`04d6303c29767b2626786262312fe6845713449f74afbc40cf96a2bbd50cee4f`

Campaign signature:

`7594e3e674c32826fdb4195244612fff4157b1f4513c83609b076246dce77a6c804a3e75aab849d38e188529ac33adf2707d1887569aa5ef5e74601ab0d08501`

## Final hostile/fail-closed checks

Targeted Pass120 tests rerun offline with Rust 1.98.1 after the VM certification:

- `destructive_campaign_requires_complete_matching_evidence` — **PASS**
  - incomplete destructive campaign is rejected;
- `destructive_campaign_certificate_is_strictly_authenticated` — **PASS**
  - signed evidence verifies, then mutation of campaign identity is rejected;
- `current_mount_evidence_is_resolvable_and_supported_profiles_fail_closed` — **PASS**
  - a nonmatching live mount is not admitted as ext4/xfs supported profile.

Pass120 pre-certification full workspace gate remains:

- `cargo check --workspace --all-targets` — PASS
- strict workspace Clippy — PASS
- full workspace tests — **766 passed / 0 failed / 8 ignored**

## Closure statement

Problem #13 required real destructive evidence in addition to the already-complete software/runtime assurance layer. That missing evidence now exists for the declared virtualized profile, including all seven required hard-power cuts, reboot/recovery verification, cryptographically authenticated evidence, exact platform-fingerprint matching, and certified-store create/open admission.

Therefore **Historic #13 may be marked PROD CLOSED for this support profile**.

A future bare-metal NVMe/SATA profile, XFS profile, different kernel, different QEMU/cache/device configuration, or other storage stack requires its own destructive campaign and fingerprint/certificate. Those are additional support certifications, not reopenings of the architecture problem.
