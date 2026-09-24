# IMPLEMENTATION REPORT — Pass75

Status: **VERIFIED**.

Pass75 deliberately avoids the general multiway/JOIN-mathematics frontier while that area is under independent R&D. The production change is confined to `kernel-plan` recovery/lifecycle policy.

## Implemented

- Added `PhysicalRecoveryPolicy` for deterministic admission of **advisor-owned** durable physical derivatives during reopen.
- Added `PhysicalRecoveryReport` exposing rebuilt, key-evaluation-budget skipped, retained-byte-budget skipped and stale/incompatible recipes.
- Relation-layout recipes and manually pinned physical recipes remain fixed durable intent and rebuild independently of advisor budgets.
- Advisor-owned I64 indexes, semantic indexes, Γ-QCN quotient factors and semantic statistics are admitted under key-evaluation and estimated-retained-byte ceilings.
- Existing `DurableRuntime::open()` remains behavior-compatible through the unlimited default policy; `open_with_recovery_policy()` exposes bounded reopen explicitly.
- `DurableRuntimeSupervisor` stores the selected recovery policy and applies it to explicit and fail-stop-triggered reopen, preventing policy loss across runtime lineage.

## Hostile corrections

An initial candidate allowed a global recovery work budget to skip manual pins. That design was rejected: a later checkpoint could then erase explicit durable physical intent. Production semantics therefore budget only advisor-owned derivatives and treat manual pins as fixed intent.

The work unit is intentionally named key evaluations rather than CPU operations. Recursive structural `CanonicalEqKey` size is not encoded in this bound. Retained-byte accounting is planning-grade physical-artifact accounting, not allocator/RSS truth.

## Tests added / strengthened

- manual pin dominates advisor recipe under zero advisor rebuild-work budget;
- optional advisor artifact may be rejected by retained-byte ceiling without changing recovered Revision;
- supervisor reuses bounded policy on repeated reopen;
- stale durable recipe is explicitly reported as incompatible while logical recovery remains exact.

## Verification

Rust `1.98.1 (48a229cea 2026-09-01)`.

Frozen source passed the complete workspace gate:

- fmt check;
- workspace/all-target check;
- debug tests: **453 passed / 0 failed / 8 ignored**;
- clippy with `-D warnings`;
- release tests: **453 / 0 / 8**;
- release build;
- rustdoc with `-D warnings`;
- overflow-checked release tests: **453 / 0 / 8**.

Freeze and post-gate `crates/` SHA-256 inventories are byte-identical.

## Ledger

Pass74 had 22 active historical OPEN. Pass75 closes concrete eager-rebuild policy/ownership/observability defects, but not the full historical recovery-economics or cross-family lifecycle items. Historical active OPEN remains **22**.
