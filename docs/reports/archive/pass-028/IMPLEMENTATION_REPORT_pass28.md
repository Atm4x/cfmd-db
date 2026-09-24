# CFMD implementation report — Pass28

Status: **SOURCE-AUDITED / NOT COMPILED**.

Pass28 source window: 2026-09-19 19:54:31–20:14:32 UTC (20m01s).
Last fully verified checkpoint remains Pass27 until Pass28 is compiled and tested with Rust 1.98.1.

## Executive state

The logical authority remains:

```text
Revision = (Schema S, SemanticEnv Γ, finite Model M)
```

Pass28 does not add a logical database primitive. It changes the physical/derived publication boundary so a logical revision transition can be prepared across authoritative `PhysicalStore` and reconstructible `MaterializedRelPlanState` before either live object advances.

## New transaction shape

```text
PhysicalStore@R ------------------------------┐
                                              │ clone + physical validation
                                              ▼
                               PreparedPhysicalStoreTransition
                                              │ exact certified handles
                                              ▼
MaterializedRelPlanState@R -------------------┤
                                              │ clone + recursive validation
                                              ▼
                              PreparedMaterializedRelPlanTransition
                                              │
                                              ▼
                               PreparedStoragePlanTransition
                                              │
                              freshness check BOTH sides
                                              │
                          extract BOTH candidates, no live writes
                                              │
                                  commit boundary
                                   /          \
                                  ▼            ▼
                        PhysicalStore@R'   PlanState@R'
```

## Production changes

### kernel-plan

- `PhysicalStore` now stores `transition_epoch` and optional bound `RevisionId`.
- private `PreparedPhysicalStoreTransition` retains exact source snapshot + candidate + certified delta;
- public `PreparedStoragePlanTransition` is the only new storage+plan commit object;
- `prepare_storage_plan_transition` prepares a one-relation source→target revision transition;
- `PhysicalStore::bind_revision` binds bootstrap state to logical revision;
- revision-bound legacy `install` / semantic delta mutation is rejected;
- reconstructible `install_i64_index` remains legal but advances epoch;
- all epoch increment paths use checked arithmetic;
- legacy unbound certified mutation itself is now candidate→swap rather than direct partial mutation.

### kernel-query

- `MaterializedRelPlanState` now stores `transition_epoch` and optional bound `RevisionId`;
- `PreparedMaterializedRelPlanTransition` stores exact source snapshot, candidate and emitted delta;
- revision-aware storage-certified preparation is candidate-only;
- unbound legacy semantic delta application is candidate→swap;
- bound legacy semantic mutation is rejected;
- `attach_storage_handles` is candidate→swap and advances epoch, preventing recursive partial attachment and invalidating stale preparations.

## Freshness law implemented

A prepared participant may be extracted only if live state still equals its exact retained source snapshot and the epoch/revision match its recorded source identity.

This intentionally strengthens the first design from merely `(revision, epoch)` to exact snapshot equality. A hostile test constructs two distinct stores with the same revision and same epoch and requires rejection.

## Commit law implemented

`PreparedStoragePlanTransition::commit`:

1. consumes the joint object;
2. validates/extracts storage candidate without mutating live storage;
3. validates/extracts maintained candidate without mutating live plan state;
4. only after both succeed replaces both live objects;
5. returns the already-computed output delta.

Thus ordinary validation errors and stale-transition errors occur before live publication. After first live replacement there are no further `Result`-returning operations.

This is an **in-memory non-crashing contract**, not a durable crash transaction. WAL/recovery must define what happens on process/filesystem failure around publication.

## Test delta

Pass27: 225 declared tests.
Pass28 source: 235 declared tests.

New hostile areas:

- invisible prepare / visible commit;
- plan-prepare failure after storage candidate creation;
- stale plan state;
- source revision mismatch;
- reconstructible storage-index mutation invalidating prepared state;
- competing prepared transitions;
- sequential revision/handle progression;
- source==target rejection;
- same revision+epoch but different source state;
- revision-bound legacy mutation bypasses, including direct `install`.

Tests are present but not executed in this container.

## Source quality audit

- 19 workspace crates;
- 29,140 Rust LOC;
- 235 declared tests;
- 0 external Cargo sources;
- 0 unsafe;
- 0 TODO/FIXME;
- 0 panic!/todo!/unimplemented! macros.

Changed production source only:

```text
crates/kernel-plan/src/lib.rs
crates/kernel-query/src/lib.rs
```

Approximate textual diff:

```text
kernel-plan:  +930 / -4
kernel-query: +197 / -7
```

Most `kernel-plan` growth is hostile integration tests.

## Verification limitation

This environment has no `cargo`, `rustc`, `rustfmt`, or `clippy-driver`. Consequently the following are **UNKNOWN, not PASS**:

- compilation;
- formatting;
- debug/release tests;
- Clippy;
- rustdoc;
- release build;
- overflow-check gate.

See `PASS28_VERIFICATION_REPORT.md` and `evidence/pass28/PASS28_TOOLCHAIN_EVIDENCE.txt`.

## Integration implications for Agent 1 (WAL)

The durable protocol should eventually align with the source/target revision transition, but should not serialize runtime `transition_epoch` or full source snapshots as semantic authority. WAL research should decide what durable prepare identity, logical mutation payload, commit marker and recovery prefix prove that revision R→R' is committed.

Derived indexes/materializations remain reconstructible.

## Integration implications for Agent 2 (semantic indexes)

A future semantic index belongs inside physical candidate state and must be pinned to its semantic module/Γ identity. Rebuilding or installing such an index may change physical freshness without changing logical revision, exactly like the current I64 index path.

## Next orchestrator frontier

After toolchain verification:

1. fix any compiler/fmt/clippy issues exposed by the real gate;
2. generalize one-relation prepare to a batch of relation mutations representing one logical revision;
3. centralize revision ownership so callers do not manually bind arbitrary ids;
4. replace full candidate/source cloning with an immutable/COW/versioned state mechanism while preserving exact stale detection;
5. tighten cross-crate capability sealing and certificate construction;
6. integrate the resulting transaction contract with Agent-1 durability and Agent-2 semantic-index findings.
