# PASS596 REPORT — REPLICATION SEMANTIC-CARRIER BRANCH HOSTILE CONSOLIDATION

## Status

PASS596 COMPLETE — CLEAN / GIT-READY checkpoint.

Official UTC start: 17:06:42
Functional source freeze: 17:18:26
Useful boundary: 17:26:42
Hard boundary: 17:30:42

## Goal

Hostile-consolidate the accumulated PASS592–PASS595 replication-authority semantic-carrier branch and the orthogonal PASS594 certified CQ integration. Add no new feature semantics. Remove stale buffered/raw routes, eliminate suppression-hidden dead code, prove the active physical authority path is singular, and establish the next repository Git-ready boundary.

## Hostile findings

1. `ReplicationAuthoritySegmentPlan::{from_frames, write_to}` remained production-visible even though production had already moved to `ReplicationAuthorityFrameSource`. They were used only by tests and preserved a misleading buffered peer API.
2. A test-only path could collect authenticated CFAO/CFAS chains back into `Vec<Vec<u8>>`, requiring a dedicated `portable_replication_authority_frames` hook. It was not a production fallback and was removed rather than retained as misleading surface.
3. `replication::authority::segments` still carried a blanket `#[cfg_attr(not(test), allow(dead_code))]`. Removing it exposed raw CFAI index codec/relocation and raw pre-CFAO segment replay as test-only substrate. They are now explicitly `#[cfg(test)]` instead of hidden by module-wide suppression.
4. `pending_single_file_frames` and `live_single_file_frames` are not stale semantic-base buffering. They represent actual incremental replication deltas and streaming checkpoint cuts and remain part of the selected live-journal law.
5. No CFAS-v1 compatibility reader/writer remains. Active segment identity is CFAS v2 only. No semantic-base full-buffer staging route or second replication evaluator was found.
6. PASS594 factorized aggregate work remains `#[cfg(test)]`; production contains certified CQ semantic interning only.

## Implemented cleanup

- Removed direct `ReplicationAuthorityFrameSource` implementations for `Vec<Vec<u8>>` and `[Vec<u8>]`.
- Removed `ReplicationAuthoritySegmentPlan::{from_frames, write_to}`.
- Updated tests to use the explicit `ReplicationAuthorityFrameSlice` adapter and the same `from_source/write_source_to` path as production.
- Removed CFAO/CFAS chain collection into buffered frames and the single-file portable-frame test hook.
- Removed the blanket non-test dead-code suppression on the replication segment module.
- Marked raw CFAI index serialization/relocation/root helpers and raw non-CFAO indexed replay as test-only.

## Selected production law after cleanup

```text
incremental live delta OR canonical semantic snapshot
    -> ReplicationAuthorityFrameSource
    -> CFAS v2 content plan (pass 1)
    -> CFAO authenticated immutable write + exact source proof (pass 2)
    -> CFLN locator/root publication
    -> same replication evaluator on recovery
```

No history fallback, buffered semantic-base route, CFAS-v1 route, alternate evaluator, or compatibility router is retained.

## Branch closure

- PASS592: exact semantic carrier instead of physical append-history transfer — CLOSED.
- PASS593: bounded-memory semantic carrier replay — CLOSED.
- PASS594: streamed semantic-base staging + certified CQ integration — CLOSED.
- PASS595: five source traversals reduced to the selected two-traversal authenticated publication law — CLOSED.
- PASS596: hostile consolidation and Git-ready branch seal — CLOSED.

PASS596 is the new Git-ready checkpoint for this branch. PASS591 remains the prior Git-ready checkpoint for the persistence-lineage branch.

## Verification

- Rust 1.98.1 retained toolchain: PASS.
- `cargo test -p kernel-durability --lib --offline`: 282 passed / 0 failed / 2 ignored.
- `cargo test -p kernel-query --lib --offline`: 166 passed / 0 failed / 10 ignored.
- `cargo test -p kernel-plan --lib --offline`: 316 passed / 0 failed / 11 ignored.
- `cargo test -p cfmd-runtime --lib --offline`: 30 passed / 0 failed.
- `cargo check --workspace --offline`: PASS.
- strict all-target no-deps Clippy over kernel-durability, kernel-query, kernel-plan and cfmd-runtime: PASS.
- `cargo fmt --all -- --check`: PASS.

## Next architecture candidate

PASS597 — Γ-factorized aggregate productionization R&D.

Start with an execution-capability bridge exposing exact retained `JointMass`/Γ-class masses without coupling the aggregate lowering to SAMF implementation identity. Before production activation, hostile-measure low fanout, high distinctness, skew, churn, self-join and multi-key distributions. Admission remains narrow and certificate-driven for exact equality-join `COUNT` and `GROUP BY join-key + COUNT`; do not generalize by analogy.

## Packaging / integrity

Repository manifest and archive integrity are sealed after this report body is frozen. The final wall-clock end and archive SHA-256 are reported with the delivered artifacts.
