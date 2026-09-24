# Support Matrix

## Certified durability profiles

### `qemu-tcg-linux-ext4-ordered-v1`

Status: **certified**.

- QEMU 8.2.2 with TCG software acceleration;
- Alpine Linux 3.24.2;
- Linux 6.18.52-0-virt x86_64;
- dedicated 512 MiB raw virtio data device;
- ext4 mounted with `data=ordered`;
- QEMU cache/AIO: `cache=none,aio=threads`;
- seven destructive hard-power-equivalent cuts passed;
- signed campaign evidence and exact platform fingerprint required by certified-store admission.

Evidence: `artifacts/certification/pass120-qemu-ext4/`.

## Not implicitly certified

The following require independent evidence before being advertised as certified durability profiles:

- bare-metal NVMe/SATA devices;
- XFS;
- other Linux kernels/distributions;
- other QEMU versions, virtual controllers or cache modes;
- Windows/macOS filesystems;
- network/distributed storage.

Functional portability and durability certification are separate claims.
