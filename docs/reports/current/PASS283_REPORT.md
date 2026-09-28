# CFMD Pass283 — typed Rust product DX foundation

Date: 2026-09-28

## Goal

Continue productization above the frozen Pass280 kernel graph. Reconcile the newest GitHub Rust-CI fixes first, then remove raw column/value plumbing from ordinary Rust application queries without creating a second database semantics.

## GitHub reconciliation

Applied the two commits newer than the prior local reconciliation:

- `81c4a8fae682e07ff4711076a7b457f92bb5a747` — namespace diagnostic support: test-only child stderr inheritance plus an explicit namespace capability diagnostic in CI;
- `bf79d39c75cd8572c43151d6d0dafd4899d1f0fe` — Ubuntu/AppArmor user-namespace enablement before the namespace preflight.

The previous `.gitignore` `core.[0-9]*` fix remains present. No older remote kernel snapshot was merged over the newer local product tree.

## P283.TYPED-RELATION — CLOSED

`cfmd-runtime` now owns a typed Rust adapter above the stable facade IR:

- `ReadContext::relation::<Marker>(RelationId)` resolves one schema-backed typed relation handle;
- `Relation::field::<T>(column)` validates the Rust scalar codec against the declared CFMD column type once;
- `Field::eq(value)` inherits the column's declared equivalence authority automatically;
- `Relation::query().filter(...).select(...)` lowers to the existing `Query` IR;
- typed selections support scalar, pair and triple projections;
- `TypedQuery` / `PreparedTypedQuery` decode directly to Rust values and retain prepare-once query authority;
- typed terminals include `all`, `first_or_none`, `one`, and `one_or_none` with product-level cardinality errors.

Raw numeric coordinates remain only at schema/generated-handle construction. Application query composition no longer repeats indices or equivalence IDs.

## P283.TYPED-WRITE — CLOSED

Added public extension traits:

- `ValueCodec` for one scalar product value;
- `RowCodec` for one complete relation row.

Built-in codecs cover unit/bool/i64/f64/String/raw Value plus tuple rows up to arity three. Generated domain structs/newtypes can implement the same traits without importing a kernel crate.

`Plan::insert_typed` / `Plan::remove_typed` accept the same typed relation handle, validate row shape against schema, and lower to the universal `Row` protocol.

## Deep-path boundary

P283 deliberately does not reinterpret an arbitrary relational join as an object/reference path. A safe deep path requires an explicit product-level reference contract containing target relation/identity and cardinality. Without that law, a seemingly object-like traversal could multiply root rows. The next DX pass should introduce this reference authority first, then lower paths through the existing relational IR.

## Verification

- `cargo test -p cfmd-runtime --all-targets --offline`: **4 passed / 0 failed**;
- `cargo test -p kernel-deployment --lib --offline`: **15 passed / 0 failed**, including Linux namespace sandbox tests;
- `cargo check --workspace --all-targets --offline`: PASS;
- strict workspace Clippy (`-D warnings`): PASS;
- strict `cfmd-runtime` rustdoc: PASS;
- `formal/lean/check_refinement.py`: PASS, **10 fault points**;
- `formal/lean/check_surface_refinement.py`: PASS;
- public typed facade continues to expose no `kernel_*` types;
- full `scripts/ci-rust.sh` rerun passed repository completeness, workspace all-targets check and strict Clippy, then hit the external 45-second tool timeout during the full workspace test build with no test failure emitted.

## Freeze

- start: **01:52:00 UTC**;
- functional freeze: **02:06:24 UTC**;
- useful boundary: **02:12:00 UTC**;
- hard boundary: **02:16:00 UTC**.

## Next target

P284 should define the first-class reference/identity/cardinality product contract and use it to implement composable typed relationship paths. Generated/static domain modules can then expose paths such as `user.passport.country.code` ergonomically while preserving exact relational/cardinality semantics. Candidate/history/watch remain subsequent runtime layers over the same facade.
