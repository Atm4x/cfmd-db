# CFMD Pass284 — object-first Rust domain model and Plan-as-result

Date: 2026-09-28
Start: 02:19:41 UTC
Useful boundary: 02:39:41 UTC
Hard boundary: 02:43:41 UTC

## Goal

Move the primary native Rust DX from relation-first handles to an object-first domain surface without creating a second database semantics. Keep relation/query IR as the universal substrate, but remove application-visible relation ids and column indices. Make write operations produce `Plan` values rather than requiring a mutable command bag as the normal API.

## Implemented

- Added `Object`, `ObjectValue`, `ObjectFieldSchema`, `ObjectSet`, `ObjectQuery`, `ObjectProjectionQuery`, and symbolic object proxies in `cfmd-runtime`.
- Added `cfmd_object!` foundation macro. One stable textual object key plus Rust fields deterministically derive product-layer relation/equivalence identities; schema construction remains fail-closed on semantic-id collisions.
- Added `SchemaBuilder::object::<T>()` and `ReadContext::objects::<T>()`.
- Object queries use generated named accessors and lower to the existing typed/untyped Query IR. No application-visible relation id or column index is required.
- `ObjectSet::insert/remove` and `ObjectQuery::update/delete` return `Plan` values. Query remains read intent; a write operation over that query yields a proposed transition.
- Added `Plan::and` for composition only when both plans originate from the exact same database snapshot.
- Added facade process-local database identity. Plans now fail closed when committed to a different open database instance even if revision/schema happen to match.
- Kept the old relation-first `Schema/Relation/Query/Plan` API as dynamic/tooling/binding substrate and escape hatch.

## Deliberately deferred

P284 does not fake object references with ordinary joins. First-class identity/lifecycle and `Ref<T>` / optional / many cardinality contracts are required before deep navigation can be safe. P285 should build that authority, then compile deep object paths to the same Query IR.

## Verification

- `cargo test -p cfmd-runtime --offline --lib --tests`: 6/6 PASS.
- strict `cargo clippy -p cfmd-runtime --all-targets --offline -- -D warnings`: PASS.
- `cargo check --workspace --all-targets --offline`: PASS.
- strict workspace Clippy: PASS.
- `formal/lean/check_refinement.py`: PASS, 10 fault points.
- `formal/lean/check_surface_refinement.py`: PASS.

## Next

P285: object identity + lifecycle-safe `Ref<T>` + `Option<Ref<T>>` / many cardinality authority, generated deep symbolic paths, and object-level `get/require` without ORM hidden I/O. Continue treating write operations as Plan producers; Candidate/preview then consumes Plan.
