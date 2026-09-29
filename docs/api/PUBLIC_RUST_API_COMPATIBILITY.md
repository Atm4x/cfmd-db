# CFMD Public Rust API Compatibility

`cfmd` is the Rust application SDK. `cfmd-runtime` and `kernel-*` are not application dependencies.

## Pre-1.0 policy

CFMD is not semver-frozen yet. Breaking public Rust API changes are allowed only when they are intentional product decisions. They must update, in the same pass:

1. `crates/cfmd/tests/public_api_contract.rs`;
2. `docs/status/PRODUCTIZATION_LEDGER.md`;
3. the relevant Rust API/README examples;
4. the pass report explaining the migration.

The contract test is deliberately source-level rather than a generated rustdoc text snapshot: it compiles a consumer against the supported root facade, object-first types, dynamic escape hatch and provenance API. Internal `cfmd-runtime`/`kernel-*` symbols are outside this compatibility promise.

## Query provenance contract

Public queries carry product-layer provenance independently of kernel node numbering:

- `QueryNodeId` is unique within the running process and is preserved by `Clone` and lowering;
- each node records `QueryNodeKind`, source file/line/column and parent node IDs;
- typed/object-first wrappers propagate `#[track_caller]` so the source is the application callsite;
- prepare/evaluate failures retain the root query node/source through `Error -> Diagnostic`.

`QueryNodeId` is diagnostic identity, not persistent database identity and must not be stored as application data.

## Object relationship values

`Ref<T>`, `Option<Ref<T>>`, and `Many<T>` are part of the public object model. A direct field such as `children: Many<Child>` is a normal supported declaration, not a schema-only marker. The persisted object row still stores only scalar/reference columns; CFMD lowers many-valued relationships into internal typed edge relations.

A relationship value materialized from a snapshot is bound to that snapshot. Merely reading the Rust field performs no I/O. I/O is explicit through `Ref::load/query` and `Many::all/load/where_/query/count/one/...`. `join`, `include`, target-side backlinks, and internal edge relation IDs are not part of the normal object-first contract.

Detached `Many::new(...)` participates in graph insertion. Bound `Many<T>` on an update means preserve the existing relationship; a newly detached `Many` value means replace it. Internal edge lifecycle is kernel-backed through live endpoint witnesses, so deleting either endpoint cannot leave a dangling current relationship fact.
