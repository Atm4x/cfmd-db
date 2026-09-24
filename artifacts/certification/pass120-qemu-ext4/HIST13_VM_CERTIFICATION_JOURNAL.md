# CFMD Historic #13 — VM durability certification journal

Append-only working journal. Scope: Pass120 `cfmd-durability-campaign` executed inside a QEMU/TCG guest against a persistent virtual block device, with the QEMU process itself killed at armed crash cuts and the same device reopened after reboot.

## Declared candidate profile

- Hypervisor/emulator: QEMU 8.2.2, TCG software acceleration (no `/dev/kvm`).
- Guest: Alpine Linux 3.24, kernel 6.18.52-0-virt x86_64.
- Data device: virtio block, raw image `/mnt/data/cfmd13vm/cfmd-data.img`, 512 MiB.
- QEMU data-device options: `format=raw,if=virtio,cache=none,aio=threads`.
- Intended filesystem: ext4, `data=ordered`.
- Host substrate: ChatGPT Linux container; the raw image itself resides on the host container filesystem. This campaign certifies only the explicitly declared virtualized profile and is not a bare-metal NVMe/SATA power-loss certification.

## Infrastructure bring-up

### I-01 — portable QEMU runtime
Status: PASS.

- Bundled `qemu-system-x86_64` and `qemu-img` execute using supplied shared libraries.
- Ubuntu loadable TCG module `accel-tcg-x86_64.so` supplied and accepted.
- QEMU reaches SeaBIOS and boots the supplied Alpine ISO under TCG.

### I-02 — guest boot and block visibility
Status: PASS.

Observed in guest:
- `/dev/vda`: 512 MiB persistent CFMD data image.
- `/dev/vdb1`: QEMU FAT host-share.
- `/dev/sr0`: Alpine Virt ISO.

### I-03 — ext4 provisioning
Status: PASS.

From the Alpine ISO package repository, `e2fsprogs` was installed offline; `/dev/vda` was formatted with `mkfs.ext4 -F` and mounted using `-t ext4 -o data=ordered`.

Filesystem UUID observed: `632c2322-b79e-11f1-80e3-d1c9cdf3a3a0`.

### I-04 — campaign executable launch
Status: OPEN → workaround in progress.

The Pass120 release campaign binary is glibc-linked while Alpine is musl. Copying the binary succeeds but direct execution reports `not found` because `/lib64/ld-linux-x86-64.so.2` is absent in the guest. This is a userspace ABI mismatch, not a filesystem/durability failure. Planned workaround: execute it through the supplied glibc loader and bundled `libc.so.6`/`libgcc_s.so.1` from the host-share.

