# Pass120 frontier — Historical #13 supported-platform real durability assurance

#13 is the final open historical problem after Pass119.

## Closure bar

Do not close #13 with process-kill tests, tmpfs/FUSE mocks, or the Lean #18 filesystem model itself.  Closure must establish the real platform profiles CFMD actually claims to support.

For each supported profile record at minimum:

1. OS/kernel and filesystem identity;
2. storage/device durability assumptions (including volatile write-cache policy where observable);
3. semantics/evidence for file `fsync`/`sync_all`;
4. parent-directory sync semantics for create/rename/remove;
5. atomic rename behavior at the publication cut;
6. power-loss/reboot recovery matrix for #18 ordinary publication;
7. power-loss/reboot recovery matrix for streaming publication/PreparedCutCapsule;
8. GC deletion + directory-sync recovery;
9. #17 external freshness authority state publication/restart;
10. explicit unsupported profiles (network FS, FUSE, filesystems/devices whose durability contract cannot be established).

## Engineering approach

Build a destructive harness that emits a monotone experiment id and expected crash cut to an independent observer, performs exactly one durability protocol step, then requires an external machine/VM/device reset at that cut.  After reboot, a verifier classifies the recovered namespace/content against the #18 admitted crash projections and #17 authority invariants.

Virtual-machine sudden-poweroff evidence may cover a specifically declared virtualized profile only if the virtual disk/cache contract is named.  It does not automatically establish bare-metal device guarantees.

Until real reset/reboot evidence is supplied for at least the claimed supported profiles, #13 remains OPEN.
