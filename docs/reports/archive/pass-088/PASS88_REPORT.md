# PASS88 REPORT

**Status:** FINAL / HISTORICAL REPAIR + CANONICAL DURABLE FORMAT MIGRATION CONVERGED

## Baseline and wall-clock boundary

Baseline: frozen Pass87 (`cfmd_workspace_pass87_restart_group_durability.zip`). Integration window started at **2026-09-22 21:26:41 UTC**. The 20-minute source boundary was **21:46:41 UTC**. The combined fmt/check/strict-Clippy/full-test gate completed by **21:43:52 UTC**; after the cutoff no production source or Cargo manifest was changed. Only reports, manifests and packaging were performed.

## Result

- Historical **#19 — PROD CLOSED**.
- Historical **#9 — PROD CLOSED**.
- All ten rows from the original portable historical-closeout bundle (#1/#2/#3/#4/#5/#7/#11/#14/#15/#19) are now production-closed.
- Production delta versus Pass87: 5 code/config files plus `Cargo.lock`:
  - `crates/kernel-plan/Cargo.toml`
  - `crates/kernel-plan/src/lib.rs`
  - `crates/kernel-query/src/lib.rs`
  - `crates/kernel-durability/src/lib.rs`
  - `crates/kernel-durability/src/store.rs`
  - `Cargo.lock`
- Frozen-tree verification: fmt/check/strict Clippy PASS; workspace tests **660 declared / 0 failed / 8 ignored**.

## #19 — bounded repair + VMF/OFC + verified observation transport

Production now has a finite `RepairCandidateProvider`, bounded search policy and explicit outcomes (`NoRepair`, unique `Prepared`, `Ambiguous`, `BudgetExceeded`). Candidate generation is untrusted policy; each candidate is accepted only through the ordinary prepared-transition path and existing VMF/OFC semantic gates.

Cross-context repair requires an explicit verified `kernel-transport` witness. The guarded query/source revision are transported into the target semantic context before comparison. A candidate with no witness is rejected; a candidate whose transported target changes the guarded observation is also rejected. Transport therefore proves comparability, not permission to change semantics.

Hostile gates cover unique repair, ambiguity, budget exhaustion, VMF rejection, OFC rejection, definitional transport, equivalent semantic-environment transport, missing-witness rejection and transported observation change. Rewrite/WritableLens remains only a candidate-provider seam.

## #9 — canonical durable-format migration registry

Durable recovery now has an explicit `CanonicalDurableState` assembly boundary after checkpoint + metadata + WAL reconciliation. Supported historical physical codecs decode into this canonical authority before a runtime store is published.

`DurableFormatRegistry` and typed `UnsupportedDurableFormat { component, version }` make unsupported outer checkpoint/metadata/manifest formats fail explicitly. A hostile generation test proves that an unsupported **highest published manifest** never causes fallback to an older generation.

`migrate_to_current_format` republishes fully recovered authority as a fresh current-format immutable generation. An end-to-end historical metadata-v6 fixture reopens through the canonical state, migrates to a new generation, and reopens with the same durable head and the unique historical defaults for fields that did not yet exist. Component-local codecs remain independent; the design deliberately does not invent one monolithic byte format.

This closes durable format migration semantics, not real-device power-loss assurance; #13 remains open.

## Next

**Pass89: historical #12 — streaming/chunked checkpoint cut, `PreparedCutCapsule`, shadow WAL and exact cut+tail publication.**

The current `Arc` revision snapshot is already a suitable immutable cut source, but Pass89 must still establish the prepare-before-cut/commit-after-cut theorem, exact frame mirroring and shadow durability watermark, one `CutId` across chunks, endpoint recovery verification, and pre/post-publication crash behavior. No #12 production source was changed in Pass88.
