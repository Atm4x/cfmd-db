# IMPLEMENTATION REPORT — Pass76

Status: **VERIFIED**.

Production source changed only in `crates/kernel-plan/src/lib.rs`.

Pass76 deliberately stayed outside the independent native-multiway/JOIN R&D line. The uploaded Γ-LCF-oriented bundle was reviewed only for forward-compatibility constraints; no code was merged.

## Implemented

- `PhysicalRecoveryReport::deferred_advisor_artifacts()` exposes compatible advisor-owned recipes rejected only by recovery budgets.
- `DurableRuntime::resume_deferred_physical_recovery()` can retry those recipes after serving has started, publishing accepted derivatives through a new immutable runtime-root version without changing the logical Revision.
- `DurableRuntimeSupervisor` exposes the same continuation surface without forcing another durable reopen.
- Recovery preflights all compatible optional recipes and schedules fixed/manual intent first; advisor-owned recipes are then ordered by increasing deterministic rebuild key-evaluation work, with stable recipe ordering only as a tie-breaker.
- Manual recipes supplied through stale/fabricated recovery reports are never replayed by continuation; incompatible advisor recipes remain fail-open derived state.

## Falsification

- A budget-zero reopen defers an advisor semantic index, online continuation rebuilds it, logical Revision remains unchanged, and the new root version is published exactly once.
- After checkpointing the resumed state, another budget-zero reopen again observes the recipe as deferrable, proving the resumed derivative remains a normal reconstructible durable recipe rather than hidden process state.
- A fabricated report containing a manual recipe and an incompatible advisor recipe cannot replay the manual pin and cannot publish a new root when nothing compatible is rebuilt.
- A two-relation hostile gives the more expensive relation the smaller `SemanticId`; with work budget 2, recovery rebuilds the two-row advisor index and skips the ten-row one, proving admission is no longer driven by identifier order.

## Boundary

This is deterministic cost-aware continuation, not benefit-ranked autonomous scheduling. Durable recipes currently do not carry a persisted workload-benefit score, and Pass76 does not invent one. Structural-key-size-aware work costing, allocator/RSS pressure, write-maintenance pricing and autonomous/background scheduling remain OPEN.

Historical active OPEN after Pass76: **22**.
