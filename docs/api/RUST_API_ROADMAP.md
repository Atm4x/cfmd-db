# Rust Product Runtime Roadmap

## Direction after Pass282

CFMD productization is **Rust-first at the semantic/runtime boundary**.

Python, .NET, Studio and other consumers must not call `kernel-*` crates directly. The stable layering is:

```text
Rust application API / language bindings
                |
                v
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
