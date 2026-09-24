# PASS87 REPORT

**Status:** FINAL / HISTORICAL RESTART + GROUP DURABILITY CONVERGED

## Baseline and wall-clock boundary

Baseline: frozen Pass86 (`cfmd_workspace_pass86_idempotency_epoch_retry_gc.zip`). Integration window started at **2026-09-22 21:00:41 UTC**. The 20-minute source window ended at **21:20:41 UTC**; the last source-changing work and final combined gate completed before that boundary. After the cutoff only reports, manifest work and packaging were performed.

## Result

- Historical **#14 — PROD CLOSED**.
- Historical **#15 — PROD CLOSED**.
- Production source delta versus Pass86: exactly 3 `.rs` files.
- Full frozen-tree verification: fmt/check/strict Clippy PASS; workspace tests **642 passed / 0 failed / 8 ignored**.

## #14 — authority-uncertainty restart classification

Checkpoint failures are no longer all treated as whole-store poison. Failures before manifest publication are known-unpublished and leave the previous root usable. From manifest rename attempt onward, publication may have changed authority and the store requires reopen. Active WAL uncertainty remains fail-stop. A poisoned reconstructible runtime mutex is recovered by rebuilding the runtime from durable authority; stale process state is not reused.

Hostile gates passed for five pre-publication fault points, two publication-uncertain points, continued checkpointing after safe abort, and explicit runtime mutex poison/recovery.

## #15 — barrier-safe group commit

Group commit accepts only a contiguous revision chain, appends PREPARE records followed by one durability barrier, appends COMMIT records followed by one durability barrier, and returns receipts only after the final barrier. Pass86 epoch-qualified retry identity and independent causal event identity are preserved per transaction.

The async batcher is non-authoritative: enqueue cannot acknowledge a commit, and a failed flush retains exact pending descriptors. Reopen reproduces the committed group outcomes. Invalid/noncontiguous/duplicate groups fail before WAL publication.

## Hostile audit of next row

Historical **#19** is the next whole-row production target. Existing Pass87 production already contains VMF/OFC primitives (`RuntimeViolationState`, `RuntimeObservationGuard`), but not the full repair closure surface. Pass88 must add a finite `RepairCandidateProvider`, unique/ambiguous/budget-exceeded result semantics, ordinary prepared-transition verification, and verified TSC observation transport for cross-context comparison. `kernel-plan` currently has no `kernel-transport` dependency, so that adapter is expected to be an explicit part of Pass88 rather than hidden coupling. No #19 production source was changed in Pass87.

## Next

**Pass88: historical #19 — bounded repair / VMF + OFC + verified observation transport.**
