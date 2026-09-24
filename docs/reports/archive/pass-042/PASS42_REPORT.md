# CFMD Pass42 Report — Composite Semantic Indexes / Mixed-Key Join / Costed Access

**Status:** VERIFIED on Rust 1.98.1.

**Scope:** data-plane planner/index cluster only. Pass42 does not change semantic authority, transaction authority, or the durable revision model.

Pass42 starts from the Pass41 typed-stateful pipeline and attacks the next connected planner/index cluster. CLOSED entries below are eliminated functional/architectural problems. Individual tests are evidence only.

## 1. Problem

Pass41 still had three directly related gaps:

1. persisted semantic indexes were single-column only, so a physical index could not represent an exact composite key such as `(TextAsciiCaseInsensitive, I64Exact)`;
2. logical/physical Join had one primary equality key but no first-class exact column-to-column filter capable of expressing and lowering additional equality predicates as one composite join key;
3. if a compatible persisted semantic index existed, direct Filter/Join paths tended to prefer it without an explicit work/selectivity decision. Pass39 had already shown that an index can lose on very small or unselective relations.

## 2. Composite persisted semantic index

`SemanticIndexBinding` is now:

```text
relation
layout
key_parts: [
    { column, equivalence },
    ...
]
```

A materialized index resolves every key part independently under pinned Γ and stores:

```text
Vec<CanonicalEqKey> -> insertion-ordered PhysicalRowId bucket
```

Consequences:

- mixed primitive equality domains are allowed in one exact key;
- each key part carries its own `SemanticId` and resolved module digest through the existing `kernel-semantic-index` binding;
- empty composite keys are rejected;
- column bounds are checked;
- Γ compatibility is checked for every component;
- relation delta maintenance computes the full composite key atomically;
- `distinct_key_count()` is available to physical access costing;
- single-column callers use `SemanticIndexBinding::single(...)` and keep the same semantics as Pass40.

This remains reconstructible physical state, not semantic authority.

## 3. Exact column-to-column equality predicate

Pass42 adds logical `RelExpr::FilterEqColumns`:

```text
FilterEqColumns {
    input,
    left_column,
    right_column,
    equivalence,
}
```

It is a normal universal relational predicate, not a physical-only special case. It has:

- typechecking against both column types;
- pinned-Γ equivalence admission/refinement checks for both input columns;
- logical evaluation;
- incremental maintained propagation;
- physical `Plan` lowering/round-trip;
- transport rewrite support;
- durable materialization metadata codec support.

The durable metadata codec uses a new query tag for this expression. Existing earlier tags remain unchanged.

## 4. Mixed/multi-key direct Join lowering

A direct two-way Join followed by one or more cross-side `FilterEqColumns` predicates can now be recognized as a composite equality join.

Example logical shape:

```text
FilterEqColumns(left.a2 == right.b2, eq2,
    JoinEq(left.a1 == right.b1, eq1))
```

with a right-side persisted index such as:

```text
[(b2, eq2), (b1, eq1)]
```

can execute as one composite semantic-index probe per left row. Index key-part order need not match logical predicate order; Pass42 explicitly maps logical keys to binding order.

The executor revalidates every returned right row against every Γ equality predicate before emitting a joined row. An inconsistent physical receipt therefore becomes an error rather than semantic authority.

This closure is intentionally limited to a **direct two-way equality Join with cross-side equality predicates**. General nested/multiway join ordering remains OPEN.

## 5. Cost/selectivity decision for existing persisted indexes

Pass42 adds `SemanticAccessCostModel` and explicit `SemanticAccessPath::{FullScan, PersistedIndex}`.

Current correctness-first work estimates are deliberately simple and deterministic:

### Filter

```text
scan_work  = row_count
index_work = 1 + matching_rows
```

The persisted index is used only when `index_work < scan_work`.

When several compatible persisted indexes cover a direct filter chain, Pass42 selects the candidate with:

1. fewer matching rows;
2. if tied, more covered key parts.

### Join

```text
nested_work  = left_rows * right_rows
average_bucket = ceil(right_rows / distinct_index_keys)
indexed_work = left_rows * (1 + average_bucket)
```

Again, the persisted index is used only when the estimated indexed work is lower.

`ExecutionStats.persisted_index_cost_rejections` makes a rejected installed index observable in tests/profiling.

This closes **scan vs already-installed persisted index** for the current direct Filter/Join physical paths. It does **not** solve whether to create, retain, rebuild, evict, or share an index; those require a broader planner/lifecycle model.

## 6. Hostile / regression evidence

Pass42 specifically verifies:

- a tiny/unselective Text semantic index is rejected by the cost model and the exact scan path is used;
- a selective Text semantic index is chosen;
- a mixed `(TextAsciiCaseInsensitive, I64Exact)` composite index drives a filter chain;
- the same composite index remains correct under relation delta maintenance;
- a mixed-key two-way Join is fused through a composite persisted right-side index;
- logical result equals fresh reference evaluation;
- Γ pinning/rebuild behavior from Pass40 remains intact;
- `FilterEqColumns` survives maintained, transport and durable metadata paths.

Targeted release evidence: `evidence/pass42/TARGETED_PASS42_RELEASE.log`.
Static audit: `evidence/pass42/STATIC_AUDIT.txt`.

## 7. Verification

Final Rust 1.98.1 gate:

```text
cargo fmt --all -- --check                                  PASS
cargo check --workspace --all-targets                       PASS
cargo test --workspace --all-targets                        PASS
cargo clippy --workspace --all-targets -- -D warnings       PASS
cargo test --workspace --all-targets --release              PASS
cargo build --workspace --release                           PASS
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps  PASS
RUSTFLAGS='-C overflow-checks=yes' cargo test ... --release PASS
```

Workspace metrics at freeze:

- 319 declared Rust tests;
- 91 `kernel-plan` tests;
- 69 `kernel-query` tests;
- 32 `kernel-durability` tests;
- 21 crates;
- 42,772 Rust LOC under `crates/`;
- 0 external Cargo sources;
- 0 `unsafe` in `crates/`.

## 8. Problem ledger

### CLOSED exactly in Pass42

1. ✅ **Persisted semantic indexes could not represent exact multi-column/mixed-primitive keys.** `SemanticIndexBinding` now supports an ordered list of `(column, equivalence)` parts and materializes a Γ-bound composite `Vec<CanonicalEqKey>` index with atomic delta maintenance.
2. ✅ **Direct two-way mixed/multi-key equality Join could not be lowered to one persisted composite semantic-index access.** `FilterEqColumns` provides the exact logical predicate and the physical executor fuses cross-side predicates with the primary Join key when a compatible composite index exists.
3. ✅ **Existing persisted semantic indexes were not subject to an explicit scan-vs-index selectivity decision.** Direct semantic Filter/Join paths now use `SemanticAccessCostModel`; tiny/unselective cases can deliberately fall back to scan and expose the rejection in execution stats.

### Partially advanced / still OPEN

1. 🟨 **Broader join planning.** Direct two-way multi-key equality Join is implemented; nested/multiway join reordering, bushy plans and general mixed structural keys remain OPEN.
2. 🟨 **Index lifecycle/costing.** Choosing between scan and an already-installed index is implemented; index creation/retention/rebuild/eviction/shared-reuse costing remains OPEN.

### Historical / active OPEN after Pass42

1. ⬜ I64 TopK residual constant-factor gap (~6–7x at Pass41 measurement).
2. ⬜ Maintained I64 Group residual constant-factor gap (~2x at Pass41 measurement).
3. ⬜ Structural/custom-equivalence canonical indexes and typed production.
4. ⬜ Nested/multiway/bushy Join planning and non-direct mixed-key plans.
5. ⬜ Cost model for **creating/retaining/rebuilding/evicting** indexes, not merely using an installed one.
6. ⬜ Shared semantic-index reuse across multiple plans/operators.
7. ⬜ Physical-index memory budgeting and rebuild scheduling.
8. ⬜ Canonical-key encoding migration/version compatibility for long-lived physical caches.
9. ⬜ Remaining physical layouts + `OrderedView`/pagination.
10. ⬜ Clone-heavy runtime transaction candidates → COW/persistent roots.
11. ⬜ Durable revision DAG / branch+merge ancestry.
12. ⬜ General historical durable-format migration.
13. ⬜ Arbitrary semantic plugin artifact packaging/signing/deployment.
14. ⬜ Transaction intent/outcome retention + GC.
15. ⬜ Streaming/chunked checkpoints/metadata.
16. ⬜ Real machine power-loss and cross-filesystem/Windows/network-FS assurance.
17. ⬜ General lock-poison/restart policy.
18. ⬜ Group commit / async durability / replication / consensus.
19. ⬜ Durable-store authentication/MAC and formal crash proof.
20. ⬜ Transaction repair runtime / distribution / formal mechanization.

## 9. Result / next direction

Pass42 closes the old single-key physical-index boundary and introduces a real, explicit access-path decision without pretending that a tiny heuristic is a complete optimizer.

The next high-value planner work is now higher-level rather than another index container:

1. index lifecycle/advisor policy — create/retain/share/evict/rebuild;
2. nested/multiway join-order search using actual relation/index statistics;
3. only then deeper multi-layout/OrderedView planning.

Residual I64 Group/TopK constant factors remain tracked independently and are not hidden by the planner work.
