# CFMD Pass195 — kernel-query hostile wave 1: indexed Group planning + blocker batch maintenance

Date: 2026-09-26
Start: 22:24:36 UTC
Useful boundary: 22:44:36 UTC
Hard boundary: 22:48:36 UTC

## Goal

Begin the full hostile cleanup of the `kernel-query` monolith. The ordering is deliberate: close correctness/architecture/mathematical/asymptotic defects first; do **not** mechanically split `src/lib.rs` while substantive hostile seams remain. Legacy/Obsolete scope remains untouched.

## P195.Q1 — indexed generic Group planner ignored its own semantic index — CLOSED

The generic maintained Group path collected all affected keys with linear semantic deduplication and then rescanned the entire delta batch for every affected group. For a batch touching `d` distinct groups this produced accidental O(d^2) planning even when the state already owned an `i64_lookup` or canonical `semantic_lookup`.

The indexed generic path now partitions the validated delta exactly once by an owner-local `IndexedGroupKey` (`I64` or canonical semantic key) using a `BTreeMap`. Each affected group is then planned only from its own entries and its existing lookup. Indexed generic Group planning is therefore O(d log d + aggregate work) instead of repeated all-batch semantic matching. The non-indexable custom-equivalence path remains the correctness fallback; it is not silently assigned a fake structural key.

Count grouping under semantic text equality and ExactF64Sum grouping both passed targeted sequential recompute comparisons after the change.

## P195.Q2 — standalone Group publication bypassed the sealed total commit — CLOSED

`MaterializedRelPlanState` already sealed Group patches before publication, but public standalone `MaterializedGroupDeltaState::apply_input_delta` still called the old fallible `commit_group_patch` directly. That retained a second production publication semantics whose signature allowed recoverable failure after mutation had begun.

Standalone Group application now uses `sealed_group_v3::seal_group_patch` followed by the total `commit_sealed_group_patch`, matching the maintained-plan publication rule. The old fallible commit machinery is now `#[cfg(test)]` only and remains solely as an oracle for sealed-commit regression comparison.

## P195.Q3 — AntiJoin blocked-key batch removals could become quadratic — CLOSED for canonical row semantics

`AntiJoin` stored left rows per join-key bucket. Batch removal previously performed semantic linear search followed by `Vec::remove` for every removed occurrence. A large blocked bucket could therefore consume quadratic work even when the AntiJoin produced no output transition.

For row semantics with a canonical representation, removals for one join key are now collected by full canonical row class and the bucket is filtered in one pass. Missing requested multiplicity is still rejected before publication because planning operates on detached local state. Truly non-canonical custom equivalences retain the semantic search fallback rather than changing equality semantics.

Both the same-key replacement/zero-crossing blocker regression and the recursive AntiJoin recompute/atomicity regression pass.

## P195.Q4 — canonical Set insertion duplicate detection — CLOSED

The indexed Scan relation mutation planner detected duplicates among inserted Set rows with `inserted_keys.iter().any(...)`, producing O(m^2) duplicate validation for a canonicalizable batch of `m` inserts. It now uses a `BTreeSet<CanonicalRowKey>`, reducing this to ordered-set membership while preserving Γ equality.

## Hostile status after wave 1

The monolith is **not yet declared clean and must not be split yet**. This pass covered the first high-risk maintained mutation areas and established concrete fixes, but the hostile program remains OPEN over the rest of `kernel-query`.

Next hostile focus before any mechanical split:

- **OPEN P196 first target:** compressed `DeltaView` weights are arbitrary `i64` (`CompactDelta::one` accepts them directly), while generic Group Count/ExactF64Sum and ordered TopK mutation paths still contain loops proportional to `|weight|`. This is therefore not merely theoretical cardinality work: a tiny-support delta can request enormous iteration. Bulk weighted aggregate/state transitions must be designed before the monolith is clean;
- remaining non-canonical semantic fallbacks in Group/Join/TopK/Scan and whether any maintained owner can avoid repeated semantic search without inventing canonicalization;
- replay/recompute convenience paths versus compiled maintained paths, especially Difference/AntiJoin, to ensure no legacy path is reachable from the hot maintained execution graph;
- TopKWithTies threshold repair and semantic-order buckets for hidden rescans not output-sensitive;
- join mutation class resolution and two-sided delta multiplication, separating unavoidable output-size work from accidental lookup work;
- flat ExecGraph vs debug recursive oracle duplication/ownership, including post-commit debug failure ordering;
- visibility/owner boundaries only after the semantic/asymptotic hostile layer is exhausted.

No mechanical file split was performed in Pass195.

## Verification

- `cargo check -p kernel-query --tests --offline`: PASS;
- targeted semantic Group Count regression: PASS;
- targeted ExactF64Sum Group regression: PASS;
- targeted AntiJoin same-key/zero-crossing regression: PASS;
- targeted recursive AntiJoin recompute/atomicity regression: PASS;
- `cargo test -p kernel-query --lib --offline`: **113 passed / 0 failed / 1 ignored**;
- strict `cargo clippy -p kernel-query --all-targets --offline -- -D warnings`: PASS;
- `cargo check --workspace --all-targets --offline`: PASS;
- `cargo fmt --all -- --check`: PASS;
- strict rustdoc (`RUSTDOCFLAGS=-D warnings cargo doc -p kernel-query --no-deps --offline`): PASS;
- `formal/lean/check_refinement.py`: PASS, **10 fault points**;
- `formal/lean/check_surface_refinement.py`: PASS.

## Modified production scope

- `crates/kernel-query/src/lib.rs`

No external public API was intentionally changed. No Legacy/Obsolete source was modified.
