# PASS35 CRASH MATRIX

Status: **VERIFIED subprocess-kill matrix** on the current Linux/container filesystem under Rust **1.98.1**.

This matrix uses a real parent/child process boundary. A child reaches a named durability boundary, creates a test-only coordination marker, blocks, and the parent calls `Child::kill()`. The child does not run Rust destructors or normal shutdown after the kill.

The coordination marker is test-only and is never part of the durable authority protocol.

| Kill point | Expected recovered authority | Observed |
|---|---|---|
| after durable WAL PREPARE, before COMMIT | previous committed revision | PASS |
| after durable WAL COMMIT, before ACK | target revision | PASS |
| after checkpoint file sync | old generation manifest; WAL tail still reaches target head | PASS |
| after new WAL sync | old generation manifest; WAL tail still reaches target head | PASS |
| after prerequisite directory sync | old generation manifest; WAL tail still reaches target head | PASS |
| after pending-manifest sync, before rename | old generation manifest | PASS |
| after manifest rename, before publication directory sync | new manifest is visible after **process kill** on this filesystem | PASS |
| after publication directory sync | new generation | PASS |
| before obsolete-generation removal | active generation survives | PASS |
| after one obsolete artifact removal | active generation survives and reopens | PASS |
| after compaction directory sync | active generation survives and reopens | PASS |
| after durable COMMIT inside runtime commit, before runtime root publish | restart rebuilds target revision from durable WAL | PASS |

## Repeated stress

`evidence/pass35/CRASH_STRESS.log` records:

- 20 repeated executions of the `kernel-durability` subprocess-kill matrix;
- 10 repeated executions of the end-to-end durable-COMMIT-before-publish restart scenario.

All repetitions passed.

## What this proves

For the tested OS/filesystem/process model:

1. uncommitted PREPARE never advances recovered semantic authority;
2. completed durable COMMIT does advance recovered authority even if the process dies before ACK/runtime publication;
3. manifest rename is the process-visible generation authority transition;
4. pre-rename checkpoint/WAL/pending files remain non-authoritative;
5. partial deletion of obsolete generations cannot delete the active generation under the current compaction selector;
6. recovery does not depend on the dead process having published its in-memory root.

## What this does NOT prove

This is **not** a power-loss proof. In particular:

- a process kill does not emulate storage-controller volatile caches;
- process kill after `rename` but before parent-directory `fsync` cannot prove that the rename survives sudden machine power loss;
- results are not evidence for Windows, network filesystems, FUSE, or filesystems with different durability contracts;
- client retry semantics after COMMIT-before-ACK remain ambiguous because no durable client transaction/idempotency key exists yet.
