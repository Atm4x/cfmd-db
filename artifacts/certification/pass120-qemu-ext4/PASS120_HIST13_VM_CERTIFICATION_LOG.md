# PASS120 Historic #13 — VM Certification Log

Append-only execution journal for the destructive VM certification campaign.

## Scope

Target: validate the Pass120 durability campaign on an isolated virtual block device using real guest Linux filesystem semantics and **hard VM power cuts** (host kills the QEMU process), followed by fresh-boot verification.

Initial certification profile under construction:

- hypervisor/emulator: QEMU 8.2.2, TCG (no KVM)
- guest: Alpine Linux 3.24.2 virt ISO
- guest kernel: Linux 6.18.52-0-virt x86_64
- data device: dedicated 512 MiB raw virtio block device
- intended filesystem: ext4, `data=ordered`
- QEMU data-disk cache mode: `cache=none,aio=threads`
- network: disabled (`-nic none`)
- test binary: Pass120 `cfmd-durability-campaign`, release build, Rust 1.98.1, built offline

This VM profile is evidence only for the named virtualized stack. It does not claim bare-metal NVMe/SATA power-loss behavior.

## Incremental journal

### 2026-09-23 — Environment/bootstrap

1. **QEMU bundle validation — CLOSED**
   - Initial bundle was missing shared libraries; supplemental library bundle supplied.
   - Initial QEMU bundle was also missing Ubuntu's loadable TCG accelerator module; supplemental `accel-tcg-x86_64.so` supplied.
   - Final result: `qemu-system-x86_64` and `qemu-img` start successfully offline; TCG loads.

2. **Rust toolchain — CLOSED**
   - Rust 1.98.1 unpacked locally and verified.
   - Pass120 `cfmd-durability-campaign` release binary built fully offline.
   - Resulting binary size: approximately 808 KiB.

3. **Host probe — EXPECTED FAIL-CLOSED**
   - Host filesystem is overlayfs.
   - Pass120 certification probe rejects it; host environment is therefore not mis-certified as ext4/xfs.

4. **Guest boot — PASS**
   - SeaBIOS -> Alpine ISO -> Linux guest completes boot under QEMU TCG.
   - Observed guest kernel: `6.18.52-0-virt`.
   - Persistent data device visible as `/dev/vda` (512 MiB).
   - Shared host/guest FAT device visible as `/dev/vdb1`.

5. **Guest ext4 provisioning — PASS**
   - Alpine live ISO contains `e2fsprogs` packages.
   - Installed `e2fsprogs` transiently from the ISO with `--force-non-repository --allow-untrusted` (no network).
   - `/dev/vda` formatted successfully as ext4.
   - Filesystem UUID observed: `632c2322-b79e-11f1-80e3-d1c9cdf3a3a0`.
   - Mounted with `data=ordered`.

6. **Campaign binary transfer — PARTIAL / runtime ABI issue found**
   - Binary copied successfully from shared FAT to ext4 and marked executable.
   - Direct execution returned `not found` despite the file existing.
   - Root cause: campaign binary is GNU/glibc-linked while Alpine uses musl; ELF interpreter `/lib64/ld-linux-x86-64.so.2` is absent in guest.
   - This is a guest runtime packaging issue, not a CFMD durability failure.
   - Next action: provide the already-available glibc loader + `libc.so.6` + `libgcc_s.so.1` through the shared FAT and invoke the binary explicitly via that loader. No new user upload is needed.

### 2026-09-23 — Fast minimal guest and supported-profile probe

7. **Minimal certification guest — PASS**
   - Switched boot controller to direct Alpine kernel/initramfs with `rdinit=/bin/sh`.
   - This bypasses OpenRC/userspace services but keeps the same Linux kernel, virtio block device and ext4 implementation.
   - Required modules (`virtio_blk`, `ext4`, `vfat`) are loaded explicitly.
   - Boot-to-controller-shell is approximately 3–4 seconds under TCG.

8. **Official Pass120 ext4 profile probe — PASS**
   - `cfmd-durability-campaign probe ext4 /mnt/cfmd/campaign-case` executed inside the guest through the bundled glibc loader.
   - Observed:
     - profile: `LinuxExt4Ordered`
     - kernel: `6.18.52-0-virt`
     - filesystem: `ext4`
     - mount source: `/dev/vda`
     - mount options: `["rw", "relatime"]`
     - super options: `["rw", "data=ordered"]`
   - Campaign returned `PROFILE_OK` and exit status 0.

The destructive matrix is now armed to run. For each case the host waits for the exact `ARMED cut=...` marker and then sends SIGKILL to the **QEMU process**, not to the campaign process. This is the VM hard-power event required by the harness.

### Destructive hard-power matrix

#### Case 1/7 — `after-candidate-file-sync`

- `2026-09-23T22:41:15.722153+00:00` ARM: PASS — exact `ARMED cut=after-candidate-file-sync` marker observed. Log: `cut01_arm_after-candidate-file-sync.log`.
- `2026-09-23T22:41:15.722255+00:00` HARD CUT: PASS — QEMU process SIGKILLed immediately after ARMED; controller elapsed `4.182006597518921` s.
- `2026-09-23T22:41:20.132910+00:00` REBOOT PROFILE: PASS — fresh VM boot re-admitted `LinuxExt4Ordered`.
- `2026-09-23T22:41:20.132979+00:00` VERIFY: PASS — `DESTRUCTIVE_CASE_PASS cut=after-candidate-file-sync` observed. Log: `cut01_verify_after-candidate-file-sync.log`; controller elapsed `3.754720449447632` s.

#### Case 2/7 — `after-prerequisite-directory-sync`

- `2026-09-23T22:41:43.026914+00:00` ARM: PASS — exact `ARMED cut=after-manifest-rename` marker observed. Log: `cut04_arm_after-manifest-rename.log`.
- `2026-09-23T22:41:43.026992+00:00` HARD CUT: PASS — QEMU process SIGKILLed immediately after ARMED; controller elapsed `3.9702205657958984` s.

### Controller-timeout recovery / journal correction

The first aggregate controller was externally timed out while case 4 verification was booting. Raw per-case serial logs remained intact. The earlier `Case 2/7` section above is therefore incomplete/misaligned and is superseded by this append-only correction; no prior lines are deleted.

- **Case 2 — `after-prerequisite-directory-sync`: PASS from raw logs.**
  - arm log contains `PROFILE_OK`, exact `ARMED cut=after-prerequisite-directory-sync`, `HOST_HARD_CUT`, and QEMU exit `-9`;
  - fresh verify log contains `PROFILE_OK`, `DESTRUCTIVE_CASE_PASS cut=after-prerequisite-directory-sync`, `VERIFY_RC=0`.
- **Case 3 — `after-pending-manifest-sync`: PASS from raw logs.**
  - arm log contains `PROFILE_OK`, exact `ARMED cut=after-pending-manifest-sync`, `HOST_HARD_CUT`, and QEMU exit `-9`;
  - fresh verify log contains `PROFILE_OK`, `DESTRUCTIVE_CASE_PASS cut=after-pending-manifest-sync`, `VERIFY_RC=0`.
- **Case 4 — `after-manifest-rename`: PASS.**
  - arm log contains exact `ARMED` followed by QEMU SIGKILL;
  - the first verify boot was interrupted by the outer controller timeout before completion;
  - a fresh retry boot subsequently produced `PROFILE_OK`, `DESTRUCTIVE_CASE_PASS cut=after-manifest-rename`, `VERIFY_RC=0`.
- **Case 5 first attempt: INVALIDATED / REPEAT REQUIRED.**
  - arm and hard-cut markers were obtained, but a stale background QEMU process from an abandoned experimental controller acquired the disk before the intended verifier. Because that intermediate mount could perform ext4 journal recovery, this attempt is not counted as certification evidence.
  - Case 5 will be re-armed from a clean state and repeated.

Current trustworthy matrix state: **4/7 PASS** (cases 1–4).

### 2026-09-23 — Harness normalization / authoritative rerun

The earlier exploratory controller used `/mnt/cfmd/campaign-case` and the first draft journal associated one heading with the wrong log name. Those exploratory records are retained above for auditability, but they are **not** the authoritative matrix.

A single minimal-initramfs controller is now used for the authoritative rerun of all seven cases. It embeds the exact Pass120 campaign binary and its glibc runtime, boots the same Alpine `6.18.52-0-virt` kernel, loads only `virtio_blk`/ext4 dependencies, mounts the same persistent `/dev/vda` raw image as ext4 `data=ordered`, and uses a distinct `/mnt/cfmd/cases/<cut>` directory per cut.

- Minimal initramfs SHA-256: `36b6626aa779e030d40dbfe1fa5a1ede98af3872eeed9739e7a78f188090cc68`.
- Guest kernel SHA-256: `40f620bc8c93d952e57dd8dfc0f94fca1759d192a4fc4a260705d50ca378559c`.
- Campaign binary SHA-256: `4fd5ef9578f31ffd17c3d124b75ad8a949bb821a58102a17fc7ef787f0db0373`.
- Control `probe`: PASS in 4.293 s. Exact observed mountinfo: ext4 `/dev/vda`, `rw,relatime`, super options `rw,data=ordered`; official campaign emitted `PROFILE_OK`.

#### Authoritative Case 1/7 — `after-candidate-file-sync`

- ARM: PASS — official campaign emitted exact `ARMED cut=after-candidate-file-sync`.
- HARD CUT: PASS — host sent SIGKILL to the QEMU process immediately after observing ARMED; controller result `HARD_CUT_SENT`, elapsed 4.248 s.
- REBOOT: PASS — fresh QEMU instance mounted the same `/dev/vda`; ext4 reported journal recovery complete.
- PROFILE RE-ADMISSION: PASS — official campaign again emitted `PROFILE_OK` for `LinuxExt4Ordered` on `/dev/vda` with `data=ordered`.
- VERIFY: PASS — official campaign emitted `DESTRUCTIVE_CASE_PASS cut=after-candidate-file-sync`; elapsed 4.396 s.
- Evidence logs: `logs/min-arm-after-candidate-file-sync.serial.log`, `logs/min-verify-after-candidate-file-sync.serial.log`.

#### Authoritative Case 2/7 — `after-prerequisite-directory-sync`

- ARM: PASS — exact `ARMED cut=after-prerequisite-directory-sync` observed.
- HARD CUT: PASS — QEMU process SIGKILLed immediately after ARMED; elapsed 4.160 s.
- REBOOT: PASS — same raw `/dev/vda` reopened; ext4 journal recovery completed.
- PROFILE RE-ADMISSION: PASS — `LinuxExt4Ordered`, `/dev/vda`, `rw,data=ordered`.
- VERIFY: PASS — `DESTRUCTIVE_CASE_PASS cut=after-prerequisite-directory-sync`; elapsed 4.336 s.
- Evidence logs: `logs/min-arm-after-prerequisite-directory-sync.serial.log`, `logs/min-verify-after-prerequisite-directory-sync.serial.log`.

#### Authoritative Case 3/7 — `after-pending-manifest-sync`

- ARM: PASS — exact `ARMED cut=after-pending-manifest-sync` observed.
- HARD CUT: PASS — QEMU process SIGKILLed immediately after ARMED; elapsed 4.235 s.
- REBOOT: PASS — same raw `/dev/vda` reopened; ext4 journal recovery completed.
- PROFILE RE-ADMISSION: PASS — `LinuxExt4Ordered`, `/dev/vda`, `rw,data=ordered`.
- VERIFY: PASS — `DESTRUCTIVE_CASE_PASS cut=after-pending-manifest-sync`; elapsed 4.126 s.
- Evidence logs: `logs/min-arm-after-pending-manifest-sync.serial.log`, `logs/min-verify-after-pending-manifest-sync.serial.log`.

#### Authoritative Case 4/7 — `after-manifest-rename`

- ARM: PASS — exact `ARMED cut=after-manifest-rename` observed.
- HARD CUT: PASS — QEMU process SIGKILLed immediately after ARMED; elapsed 4.429 s.
- REBOOT: PASS — same raw `/dev/vda` reopened; ext4 journal recovery completed.
- PROFILE RE-ADMISSION: PASS — `LinuxExt4Ordered`, `/dev/vda`, `rw,data=ordered`.
- VERIFY: PASS — `DESTRUCTIVE_CASE_PASS cut=after-manifest-rename`; elapsed 4.315 s.
- Evidence logs: `logs/min-arm-after-manifest-rename.serial.log`, `logs/min-verify-after-manifest-rename.serial.log`.

#### Authoritative Case 5/7 — `after-manifest-directory-sync`

- ARM: PASS — exact `ARMED cut=after-manifest-directory-sync` observed.
- HARD CUT: PASS — QEMU process SIGKILLed immediately after ARMED; elapsed 4.231 s.
- REBOOT: PASS — same raw `/dev/vda` reopened; ext4 journal recovery completed.
- PROFILE RE-ADMISSION: PASS — `LinuxExt4Ordered`, `/dev/vda`, `rw,data=ordered`.
- VERIFY: PASS — `DESTRUCTIVE_CASE_PASS cut=after-manifest-directory-sync`; elapsed 4.260 s.
- Evidence logs: `logs/min-arm-after-manifest-directory-sync.serial.log`, `logs/min-verify-after-manifest-directory-sync.serial.log`.

#### Authoritative Case 6/7 — `after-obsolete-remove`

- ARM: PASS — exact `ARMED cut=after-obsolete-remove` observed.
- HARD CUT: PASS — QEMU process SIGKILLed immediately after ARMED; elapsed 4.393 s.
- REBOOT: PASS — same raw `/dev/vda` reopened; ext4 journal recovery completed.
- PROFILE RE-ADMISSION: PASS — `LinuxExt4Ordered`, `/dev/vda`, `rw,data=ordered`.
- VERIFY: PASS — `DESTRUCTIVE_CASE_PASS cut=after-obsolete-remove`; elapsed 4.278 s.
- Evidence logs: `logs/min-arm-after-obsolete-remove.serial.log`, `logs/min-verify-after-obsolete-remove.serial.log`.

#### Authoritative Case 7/7 — `after-obsolete-directory-sync`

- ARM: PASS — exact `ARMED cut=after-obsolete-directory-sync` observed.
- HARD CUT: PASS — QEMU process SIGKILLed immediately after ARMED; elapsed 4.515 s.
- REBOOT: PASS — same raw `/dev/vda` reopened; ext4 journal recovery completed.
- PROFILE RE-ADMISSION: PASS — `LinuxExt4Ordered`, `/dev/vda`, `rw,data=ordered`.
- VERIFY: PASS — `DESTRUCTIVE_CASE_PASS cut=after-obsolete-directory-sync`; elapsed 4.207 s.
- Evidence logs: `logs/min-arm-after-obsolete-directory-sync.serial.log`, `logs/min-verify-after-obsolete-directory-sync.serial.log`.

### Destructive matrix summary

Authoritative rerun result: **7 / 7 cuts PASS**. Every case observed the official `ARMED` marker, then killed the entire QEMU VM process, then booted a fresh QEMU instance over the same raw virtio block device, re-admitted the exact ext4 supported profile, and observed the official `DESTRUCTIVE_CASE_PASS` marker after ext4 recovery.

This satisfies the VM hard-power portion of Pass120 for the named virtualized profile. Remaining Pass120 closure steps are evidence aggregation/signing and certified-store token acceptance on the exact matching platform fingerprint.


### Final signed-evidence / certified-store admission — PASS

After the authoritative 7/7 destructive matrix, the aggregated evidence was signed and consumed on a fresh boot of the exact same declared profile.

- evidence digest: `e651425e80bc4d837544109637944a0e85c265c197dbc568d089fc180980b7d5`
- platform fingerprint: `e96b6014187ffdece8bb6b310456a533d3fad39f1b86a7987441fa4b6d25db92`
- campaign signature authentication: **PASS** (`CERTIFICATE_VERIFY=PASS`)
- certified store create/open: **PASS** (`CERTIFIED_STORE_CREATE_OPEN=PASS`)
- certifier exit marker: **PASS** (`CFMD_CERTIFY_RC=0`)
- final targeted fail-closed tests: **3/3 PASS**

**Historic #13 conclusion:** PROD CLOSED for the explicitly certified QEMU 8.2.2 TCG / Linux 6.18.52-0-virt / raw virtio / ext4 data=ordered / cache=none,aio=threads virtualized profile. This is not a bare-metal NVMe/SATA certification.
