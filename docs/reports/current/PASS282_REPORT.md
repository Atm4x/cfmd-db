# CFMD Pass282 — Rust product create/schema boundary

Date: 2026-09-28
Start: 01:35:01 UTC
Useful boundary: 01:55:01 UTC
Hard boundary: 01:59:01 UTC

## Goal

Continue the Rust-first product phase above the frozen kernel graph. P282 makes a durable CFMD database creatable and introspectable without importing or constructing any `kernel-*` type, while keeping the low-level product boundary small enough to serve native Rust and future language bindings.

## GitHub reconciliation

Two commits newer than the uploaded Pass280 baseline were inspected before product work:

- `06ff2b06838f2e6ed8a82e5eb966733047130a49` (`Fix for CI`) exposed the root cause of missing Rust `core.rs` files: `.gitignore` used `core.*`, which also ignores source files named `core.rs`. The repository policy is now `core.[0-9]*` for numbered crash dumps. The remote commit also restored four omitted `core.rs` sources; those sources already existed in the local Pass281 tree, so they were not overwritten from GitHub.
- `cc0ba594f4ea1deab93618e0cbb1c1ef30181b55` (`idk`) changes retained-evidence hashes in `REPOSITORY_MANIFEST.sha256`; no product/CI semantics were imported. P282 regenerates its manifest from its own final tree.

No older remote kernel implementation replaced the newer local Pass281 kernel/product baseline.

## P282.CREATE — facade-owned durable creation — CLOSED

Added `Database::create(path, schema)` to `cfmd-runtime`.

The public caller no longer constructs:

- `kernel_schema::Schema` / `SemanticEnvironment`;
- `kernel_semantics::SemanticRegistry`;
- `kernel_plan::PhysicalStore` / `RuntimeRevisionBundle`;
- `kernel_revision::Revision`.

The facade compiles its own schema definition once into those internal authorities and hands the resulting root to the existing durable runtime.

## P282.SCHEMA — universal low-level product schema — CLOSED

Added facade-owned:

- recursive `Type` covering scalar/product/sum/option/set/bag/seq/map/guarded recursion;
- `ScalarType`;
- primitive `PrimitiveEquivalence` and `PrimitiveOrdering` contracts;
- `RelationSchema::{bag,set}`;
- `SchemaBuilder` / `Schema`;
- `SchemaView` through `ReadContext::schema()`.

Semantic IDs are rejected if reused across facade semantic namespaces. Kernel types remain implementation-private.

## P282.EMPTY-TYPED — no sentinel bootstrap — CLOSED

The first implementation used an empty untyped row-store and the E2E gate immediately exposed `PhysicalTypeMismatch` on first insert. P282 does not work around this with seed/sentinel rows.

Creation now derives the declared kernel column types once and uses `NativeRelation::typed_from_rows(&[], column_types)` to create a genuinely empty but physically typed relation. This works through the existing algebraic native column machinery and introduces no threshold/type-specific facade routing.

## P282.DX / layering law

The stable low-level product layer stays data-oriented and binding-friendly. Higher-level generated/typed Rust APIs should lower to this same surface rather than becoming parallel query/write semantics.

Target layering:

```text
idiomatic/generated Rust DX     Python/.NET/Studio
             \                  /
                 cfmd-runtime
                      |
                 kernel graph
```

The next Rust work is typed relation/domain handles and field/path query construction above the existing universal IR.

## Regression

The end-to-end product test now imports only `cfmd-runtime` and `std`. It performs:

1. facade schema build;
2. `Database::create` on an empty typed relation;
3. schema introspection;
4. Plan insert + durable commit;
5. prepared query execution;
6. transaction idempotency / conflict checks;
7. close;
8. `Database::open`;
9. persisted query verification.

A second regression verifies cross-namespace semantic-ID collision rejection.

## Verification before freeze

- `cargo test -p cfmd-runtime --all-targets --offline`: PASS, **2 integration tests**;
- `cargo check --workspace --all-targets --offline`: PASS;
- strict workspace Clippy (`-D warnings`): PASS;
- strict `cfmd-runtime` rustdoc (`-D warnings`): PASS;
- public `kernel_*` signature leakage scan: CLEAN (`pub(crate)` implementation details excluded);
- `formal/lean/check_refinement.py`: PASS, **10 fault points**;
- `formal/lean/check_surface_refinement.py`: PASS;
- `cargo fmt --all -- --check`: PASS after final formatting.

Kernel source remains frozen; P282 changes the product facade, documentation, Cargo dependency placement required by the facade, Git ignore policy and manifest.

## Next target — P283

Build idiomatic typed Rust DX above the universal runtime IR:

1. static/generated relation/domain handles;
2. typed field/path expressions instead of raw column indices;
3. typed projection/result shaping;
4. deep relationship paths without exposing joins in ordinary application code;
5. preserve the existing `Query`/`Plan` low-level protocol as the single binding authority.

Candidate/history/watch follow only after this native Rust surface is coherent.
