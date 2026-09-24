# CFMD Pass104 — Stage 6 kernel/allocation checkpoint

## Status

Stage 6 remains **OPEN**. Historical #21/#22 remain **not PROD CLOSED**.

## What this pass proved/fixed

1. Re-read `CFMD_RND_EXECGRAPH_BLOCKERS_V3_CONVERGED_2026-09-23`: its stable production role is a general scheduler (`PreparedRelGraph` / `CompiledTransitionProgram` / `GraphPatchSet`) above the existing V5 plan/commit kernels, not a Group-specific replacement. No dead graph metadata was merged in this pass.
2. Localized the Pass103 ~108k allocations/update by operator prefix. Scan/Filter/Project/Join/Group were small; adding TopK caused the explosion.
3. Removed the O(n) `BTreeMap<i64, Vec<Row>>::clone()` from the I64Rows TopK plan. Candidate output is now computed by ordered base/overlay merge and stops at the TopK/tie boundary.
4. Added compile-once canonical Scan row-position lookup and a reverse position→canonical-key index so canonical removals and swap-removal commit do not hide linear semantic scans.
5. Preserved the immutable whole-tree plan → root materialization → commit authority from Pass103.

## Measured result

Representative release probes at 50k rows after the fix:

- whole-chain allocation: ~142 allocator calls/update (Pass103: ~108,466);
- linear Join→Group→TopK chain: ~12 us/update;
- blocker→Group→TopK chain: ~10 us/update;
- TopK maintained: ~0.56 us vs ~0.68 us corrected hand baseline, full replay ~0.29 ms;
- Group remains ~5x the current hand baseline and is the principal constant-factor Stage-6 residual.

Raw outputs are under `PASS104_BENCHMARKS/`.

## Correctness gates

- `cargo fmt --all -- --check`: PASS
- `cargo check -p kernel-query --all-targets`: PASS
- strict `cargo clippy -p kernel-query --all-targets -- -D warnings`: PASS
- targeted TopK + recursive Join/Blocker tests: PASS
- `cargo test --workspace --all-targets --quiet`: PASS, 0 failures (same 692 passed / 8 ignored suite structure)

## Remaining Stage-6 frontier

1. Close or explain the Group constant-factor gap under the corrected universal comparator/workload matrix.
2. Run the final independent multi-process workload distributions and allocation matrix after that change.
3. Integrate ExecGraph V3 as one general compile-time scheduler over all operator kernels, with shadow equivalence against the recursive executor; do not specialize it to Group.
4. Only after those gates reconsider PROD CLOSED for historical #21/#22.
