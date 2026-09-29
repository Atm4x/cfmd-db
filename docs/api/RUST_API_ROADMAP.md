# Rust Product Runtime Roadmap

## Direction after Pass282

CFMD productization is **Rust-first at the semantic/runtime boundary**.

Python, .NET, Studio and other consumers must not call `kernel-*` crates directly. The stable layering is:

```text
Rust applications        language bindings
       |                         |
      cfmd                       |
       \                       /
                cfmd-runtime
                |
      stable owned product IR
                |
                v
       internal kernel crates
```

The direct Rust application API and every foreign-language binding share this one product/runtime authority. Bindings translate host-language objects to `cfmd-runtime` types; they do not reproduce query, write, Candidate, watch or history semantics.

## Pass281 foundation — IMPLEMENTED

`crates/cfmd-runtime` is the first stable anti-corruption layer above the kernel graph.

Implemented vertical slice:

- `Database::open` over the authoritative durable runtime;
- immutable `ReadContext` snapshots with stable facade `RevisionId`;
- facade-owned typed IDs (`RelationId`, `EquivalenceId`, `OrderingId`, `TypeId`, ...);
- facade-owned recursive `Value`/`Row` representation;
- facade-owned `Query` IR for scan/filter/project/join/distinct/TopK;
- `PreparedQuery` so immutable query structure compiles once per semantic context;
- `RelationResult` with Bag/Set distinction preserved;
- `Plan` with relation insert/remove mutations;
- durable atomic commit with explicit `TransactionId` and stale-revision rejection;
- stable coarse `ErrorKind` taxonomy without public kernel error types.

An end-to-end regression creates a real durable kernel runtime, closes it, then performs open → prepare → execute → plan → durable commit → execute exclusively through `cfmd-runtime`.

## Boundary laws

1. No `kernel_*` type may occur in a public `cfmd-runtime` signature.
2. No binding may depend on internal crate topology.
3. Prepared immutable query authority is owned by the Rust runtime layer, not reconstructed per FFI call.
4. Bulk rows/deltas cross language boundaries as batches, never as one FFI call per cell/row.
5. Current, historical and speculative worlds must be explicit runtime handles.
6. Candidate/watch/history semantics are implemented once in Rust and projected to language bindings.
7. Kernel refactors are allowed behind this facade without changing product semantics.
8. The lowest stable product layer stays small and data-oriented; ergonomic typed APIs are adapters above it, not parallel semantics.
9. Creating an empty typed database must not require sentinel data, manual physical layouts, or kernel imports.

## Next stages

### P282 — creation + schema product boundary — IMPLEMENTED

- facade-owned recursive `Type` and `SchemaBuilder`;
- primitive equivalence/ordering contracts owned by the facade;
- `RelationSchema::bag/set`;
- `Database::create` without kernel construction types;
- typed-empty relation bootstrap without fake seed rows;
- read-context schema introspection;
- public semantic-ID collision checks;
- E2E create → commit → close → reopen using only `cfmd-runtime`.

### P283–P291 — typed/object/Candidate Rust DX — IMPLEMENTED

Typed relation/domain handles, object-first entities/references/cardinality and the `Plan -> Candidate` preview/commit workflow are implemented while preserving the low-level facade IR as the universal binding protocol.

### P292–P295 — history / historical worlds / semantic rebase — IMPLEMENTED

`Database::history()`, exact durable inverse, `Database::at(revision)`, revision-anchored history and certified non-head undo/rebase are implemented without a second mutation pipeline or generic merge fallback.

### P296–P298 — exact watch protocol + lifecycle — IMPLEMENTED FOUNDATION

- maintained query subscription authority;
- revision-tagged exact collection/scalar deltas;
- provider-neutral wake delivery;
- explicit cancellation and deterministic runtime-close wake;
- exact lag status and sequential durable-history catch-up;
- explicit capability rejection when exact maintenance/history coverage is unavailable.

### Language bindings

Only after the runtime contracts above are stable:

- Python/PyO3 facade;
- .NET/WPF bridge;
- Studio/local tooling transport.

These are projections of the same Rust runtime protocol, not independent database implementations.


## Pass337 — public Rust application facade — IMPLEMENTED FOUNDATION

`crates/cfmd` is the only direct dependency required by ordinary Rust applications. It owns the
application-facing namespace while `cfmd-runtime` remains the universal semantic/runtime boundary
for bindings and hosted composition.

P337 establishes:

- object-first root/prelude exports for `Database`, schema, entities, Plans, Candidates, history and watch;
- `cfmd::dynamic` as the explicit relation/value/query escape hatch for tooling/generated bindings;
- a facade-owned diagnostic taxonomy (`DiagnosticCode`, `Diagnostic`, `ErrorDiagnosticExt`);
- a CI/public-surface gate that rejects direct `kernel-*` dependencies or source paths in `cfmd`;
- end-to-end application tests and an executable TODO example importing only `cfmd`;
- no async-runtime dependency in the core facade.

P337 deliberately kept async runtimes outside the core facade. P346-P348 resolve that boundary without a mandatory executor adapter: exact watches expose ordinary Rust Futures directly through `next().await`, and any executor may poll them. Executor-specific crates remain optional acceleration points rather than required semantic layers.

### Pass338 — generated Rust entity DX — IMPLEMENTED FOUNDATION

`cfmd-derive` provides `#[derive(CfmdEntity)]` and is re-exported by `cfmd`. A domain struct now
declares a stable `#[cfmd(key = "...")]` and one strongly typed `#[cfmd(id)] Id<SelfType>` field;
scalar fields plus `Ref<T>` / `Option<Ref<T>>` are lowered into the existing `Object`/`RowCodec`
contract and symbolic proxy. No second schema model or runtime semantics is introduced.

The derive validates named non-generic structs, requires exactly one typed identity, rejects implicit
`Id<T>` fields, and attaches `syn::Error` diagnostics to the offending key/field/type spans. P339 adds generated reverse-many declarations, schema-build validation of the required `via` reference and target registration, explicit relationship cardinality metadata, and additional compile-fail declaration diagnostics. Stable query-node/span identity remains follow-up work.

P345 freezes shared readiness identity and bounded durable drain. P346/P347 establish race-free standard-library `Waker` registration and hostile coverage. P348 removes the transitional async wrapper: raw/object/projection watches now expose direct executor-neutral `next().await`, and relation dependency frontiers suppress unrelated task wakes. Tokio remains only a dev compatibility executor.


## Pass283 typed Rust slice — IMPLEMENTED FOUNDATION

The first idiomatic Rust adapter now sits directly above the stable product IR:

```rust
struct Numbers;

let numbers = snapshot.relation::<Numbers>(RELATION)?;
let number = numbers.field::<i64>(0)?;
let name = numbers.field::<String>(1)?;

let query = numbers
    .query()
    .filter(number.eq(2))
    .select((number, name));

let rows: Vec<(i64, String)> = query.all(&snapshot)?;
```

The numeric coordinate remains only in schema/generated handle construction. Application query code uses typed handles. Equality semantics are inherited from the declared relation schema; callers do not repeat an equivalence id on every predicate.

Typed writes use the same relation handle:

```rust
plan.insert_typed(&numbers, (2_i64, "two".to_owned()))?;
```

`ValueCodec` and `RowCodec` are intentionally public extension points. Generated domain newtypes/records can implement them while the universal `Value`/`Row` protocol remains unchanged below.

Deep relation paths are deliberately not synthesized from arbitrary joins yet. The product schema must first expose an explicit reference target + target identity + cardinality contract; otherwise a navigation that looks object-like could silently multiply root rows. P284 should add this reference law and lower paths through the existing relational IR.

### Hosted server composition — P303

`cfmd-host::HostedServer` is the Rust composition point for hosted deployment. It accepts replaceable authentication/authorization providers, issues restricted hosted connections, bounds admission/in-flight work, and exposes canonical frame handling. Concrete transports remain separate crates/adapters.

### Hosted security lifecycle — P304

`Session` authority is live and shared across derived values. `cfmd-host` owns provider-driven refresh/revoke, explicit channel binding, expiring grants and graceful drain, while concrete schedulers/transports remain adapters. This is the security baseline for first-party IPC/TCP transport conformance.


### Pass340 — query provenance and compatibility gate

The public SDK now preserves product-layer query provenance independently from kernel node IDs. Dynamic, typed and object-first queries retain a clone-stable process-local node ID, operation kind, source location and parent IDs; query failures surface that provenance through facade diagnostics. `crates/cfmd/tests/public_api_contract.rs` is the source-level compatibility gate under the explicit pre-1.0 policy in `PUBLIC_RUST_API_COMPATIBILITY.md`.

P341 established direct `Many<T>` construction, but P342 supersedes its temporary virtual/backlink interpretation. `Many<T>` is now a real object relationship value whose internal edge representation is runtime lowering; no target backlink is required merely to declare the relationship.


## P342 — object-first relationship values

P342 supersedes the earlier reverse-many/backlink DX. `Ref<T>` / `Many<T>` are real object relationship values, while the relation/Γ representation is compiler/runtime lowering. `Many<T>` needs no target-side backlink. Materialized relation values bind to the exact snapshot but perform no I/O on field access; explicit `load/all/where_/query/count/one` operations perform evaluation. Detached object graphs lower into one Plan. Bound Many values preserve an existing relationship during update; detached Many values replace it. Internal edge rows carry live endpoint witnesses so kernel lifecycle normalization removes dangling current relationships after endpoint deletion.

P345-P348 provide runtime-neutral readiness/drain plus direct watch Future/Waker ergonomics without an executor dependency. A dedicated executor crate is not a default requirement; add one only for executor-specific capabilities proven useful by measurement.

## Pass351 — transaction composition surface

The object-first Rust facade now has an explicit snapshot-bound transaction composer:

```rust
let mut tx = db.transaction(TransactionId::new(42))?;
let todos = tx.objects::<Todo>()?;
tx.apply(todos.insert(first)?)?;
tx.apply(todos.insert(second)?)?;
let preview = tx.preview()?;
tx.commit()?;
```

`Transaction` is intentionally not a parallel transactional store. Every mutation remains an ordinary `Plan`; composition is admitted only for the exact same database snapshot and authority, preview is the ordinary Candidate preview, and commit is the ordinary exact commit. Stale publication and cross-snapshot composition fail closed. Explicit certified rebase remains a distinct operation rather than an implicit transaction fallback.

Publication does not consume the `Transaction`; retrying the unchanged transaction preserves the same durable transaction identity and returns the ordinary idempotent `AlreadyCommitted` outcome. This is required for caller-side uncertainty after publication and avoids inventing a second retry protocol.

### P352: Γ-native ordered predicates

P352 closes the equality-only field gap. `FilterOrderConst` is a first-class relational primitive, prepared against pinned Γ ordering semantics and maintained O(delta). Object fields with declared canonical order expose `greater_than/greater_than_or_equal/less_than/less_than_or_equal` plus inclusive `between`; generated schema installs stable per-field ordering IDs. Rust `Ord`, SQL-expression fallback, generic closure predicates, and post-materialization filtering are not semantic authorities. Exact object watches quotient non-matching revisions as before.
