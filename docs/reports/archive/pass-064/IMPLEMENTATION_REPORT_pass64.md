# IMPLEMENTATION REPORT — Pass64

Pass64 gives Γ-QCN endpoint factors a conservative production lifecycle policy on top of the Pass63 multi-family inventory and global retained-byte budget boundary.

## 1. Workload-driven quotient-factor lifecycle

`PhysicalStore::advise_semantic_quotient_factors` consumes prepared-plan workload samples and `PhysicalArtifactAdvisorPolicy` byte ceilings. For plans whose current physical cost model already selects Γ-QCN, the advisor computes exact endpoint bindings and read-side canonicalization work, then creates/rebuilds/retains/reuses/evicts quotient factors without changing semantic authority.

Only advisor-owned factors are evictable. Compatible manually materialized factors are reused as fixed physical state and are not silently adopted.

One-shot factor construction is rejected when build canonicalization does not amortize. Selected candidate state is reused for publication rather than built twice.

## 2. Cost model consumes factor reuse

`estimated_quotient_build_work` now observes compatible maintained factors. Exact pinned-Γ factors with coherent row cardinality contribute zero endpoint canonicalization work; stale or missing factors retain the previous row-count charge.

Execution and advice share `preferred_nway_order_preserving_join_inputs`, so the advisor does not claim savings for a Γ-QCN path that the current executor would reject.

## 3. Manual ownership transfer

Calling `PreparedPlan::materialize_semantic_quotient_factors` explicitly now removes the requested factors from advisor ownership even if no physical rebuild is required. This makes manual materialization a stable pin rather than a no-op that can later be undone by advisor eviction.

## 4. Exact replacement credit under global byte budgets

When advice replaces an incompatible manually owned semantic index or quotient factor, the old artifact's retained-byte estimate is credited before the replacement is charged. Tight global budgets therefore evaluate replacement memory rather than old+new double counting.

## 5. Hostile evidence

The Pass64 tests establish:

- repeated Γ-QCN advice creates three endpoint factors and produces 140 maintained quotient-key hits on the same logical result;
- planner quotient-build work drops to zero after compatible factors exist;
- repeated advice is a publication no-op;
- empty workload evicts only advisor-owned factors;
- explicit manual materialization pins previously advisor-owned factors;
- one-shot and byte-budget rejections are exact;
- a currently non-QCN path does not receive unused factors;
- stale manual semantic-index replacement succeeds under the exact replacement-sized global budget;
- all existing multiway Γ-QCN fixtures remain green.

## 6. Deliberate limits

This is not yet a universal physical advisor. It does not model write-rate maintenance cost or counterfactual path-shaping, and it does not create/evict Γ-QCN support, specialized I64 indexes, statistics or layout families. Those remain part of the historical multi-family lifecycle/telemetry frontier.

## 7. Verification

Frozen Pass64 source passes fmt, workspace check, full debug tests, strict Clippy, full release tests, release build, strict rustdoc and warmed overflow-check release tests under Rust 1.98.1. Source hashes match the freeze snapshot exactly.
