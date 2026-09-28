# CFMD Pass281 — Rust product runtime foundation

Date: 2026-09-28
Start: 01:09:09 UTC
Useful boundary: 01:29:09 UTC
Hard boundary: 01:33:09 UTC

## Goal

Begin productization above the Pass280-frozen kernel graph with a universal Rust boundary that future Python/.NET/Studio surfaces can bind to without importing `kernel-*` contracts directly.

## Result — `cfmd-runtime` foundation

Added workspace crate `crates/cfmd-runtime` as the external product/runtime anti-corruption layer.

Public facade-owned vocabulary now includes:

- `Database`, `ReadContext`;
- `RelationId`, `EquivalenceId`, `OrderingId`, `TypeId`, `FieldId`, `VariantTagId`, `RevisionId`, `TransactionId`;
- recursive `Value`, `EntityRef`, `Row`;
- `Query`, `PreparedQuery`, `RelationResult`, `OrderDirection`;
- `Plan`, `CommitOutcome`;
- `Error`, `ErrorKind`.

No internal kernel type appears in a public `cfmd-runtime` signature.

### Read vertical slice

Implemented:

```text
Database::open
  -> immutable ReadContext snapshot
  -> Query construction
  -> prepare once under pinned semantic context
  -> PreparedQuery::execute
  -> Bag/Set facade result
```

Supported initial relation IR: scan, equality filter, project, equi-join, distinct and TopK-with-ties.

### Write vertical slice

`Database::plan()` captures an immutable source snapshot privately inside `Plan`. Relation inserts/removals are converted to typed kernel deltas only at commit against that exact pinned semantic context.

This is important for durable idempotency: facade prechecks do not shadow the kernel retry protocol. Regression coverage proves:

- first commit -> `Committed`;
- exact same `Plan + TransactionId` retry -> `AlreadyCommitted`;
- same `TransactionId` with different intent -> `TransactionConflict`;
- stale unknown transaction remains rejected by the underlying revision transition authority.

## End-to-end product regression

The integration test constructs one real durable CFMD runtime using internal primitives only as fixture setup, drops it, then performs the product path exclusively through `cfmd-runtime`:

```text
open durable DB
-> snapshot revision 1
-> prepare + execute filter query
-> build Plan
-> durable commit
-> idempotent retry check
-> transaction-intent conflict check
-> snapshot revision 2
-> query committed rows
```

Result: PASS.

## Product sequencing change

Product implementation is now Rust-runtime first:

```text
Rust applications / Python / .NET / Studio
                  |
             cfmd-runtime
                  |
             kernel-* graph
```

`CFMD_PYTHON_FACADE_THEORY.md` remains the detailed Python UX target, but Python does not implement database semantics or call kernels directly.

Updated current documentation:

- `README.md`;
- `SPEC.md`;
- `CHANGELOG.md`;
- `docs/api/RUST_API_ROADMAP.md`;
- `docs/api/PRODUCT_ROADMAP.md`;
- `docs/api/CFMD_PYTHON_FACADE_THEORY.md` implementation-order note;
- `docs/status/PROJECT_STATUS.md`;
- `docs/status/REPOSITORY_LAYOUT.md`;
- `docs/architecture/ARCHITECTURE.md`;
- `docs/architecture/CRATE_MAP.md`;
- append-only `docs/spec/CFMD_CORE_SPEC.md` product-boundary handoff.

Workspace now contains 28 crates: 27 internal kernel/infrastructure crates plus `cfmd-runtime`.

## Verification

Passed:

- `cargo test -p cfmd-runtime --offline`: 1 end-to-end integration test PASS;
- `cargo check --workspace --all-targets --offline`: PASS;
- strict `cargo clippy --workspace --all-targets --offline -- -D warnings`: PASS;
- strict `cargo doc -p cfmd-runtime --no-deps --offline` with `RUSTDOCFLAGS=-D warnings`: PASS;
- `cargo fmt --all` / formatting gate: PASS;
- public-signature leakage scan: no `kernel_*` types in exported facade signatures;
- `formal/lean/check_refinement.py`: PASS, 10 fault points;
- `formal/lean/check_surface_refinement.py`: PASS.

The full `cargo test --workspace --lib` attempt did not report a semantic/test failure; it was interrupted by the sandbox filesystem limit while Rust tried to write an incremental `kernel-plan` query cache (`No space left on device`). `target/` was removed immediately. Pass280 kernel sources and formal sources were not modified by P281; the new facade itself is covered by its real durable end-to-end test plus workspace check/Clippy.

The wrapper `scripts/ci-formal.sh` could not run because this sandbox instance has no Lean binary installed. Its two source-refinement checkers were run directly and passed; formal/kernel sources are unchanged from the previously verified Pass280 baseline.

## Next target — P282

Make creation/schema ownership product-grade rather than exposing kernel construction primitives:

1. facade-owned scalar/type/schema/relation builders;
2. facade semantic module/equivalence declarations needed by ordinary applications;
3. `Database::create` from facade schema/state input;
4. schema introspection and typed relation handles;
5. stronger domain-oriented validation/error mapping.

Only after the Rust create/query/write boundary is coherent should Candidate/history/watch be layered on top, followed by language bindings.

## Freeze

Functional freeze: **01:25:58 UTC**. After this point only manifest verification and packaging are performed.
