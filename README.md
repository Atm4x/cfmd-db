# CFMD

CFMD is an experimental embedded/local database kernel built around a constructive finite-model view of state. The repository contains the Rust kernel, revision/change/query machinery, durability/recovery, trust/deployment layers, executable law tests, and Lean artifacts that bind selected architectural claims to the implementation.

## Current status

The historical kernel backlog (#1–#22) is closed for the declared support scope, and the global hostile/refactor campaign is **COMPLETE / FROZEN after Pass280**. Every kernel crate has received either a dedicated hostile closure pass or a grouped audit proportional to its size; the historically heavy `query/plan/semantics/durability` line was revalidated against the final workspace before freeze.

`FROZEN` does not mean “bug-free forever”. It means kernel cleanup is no longer continued by inertia: reopen a frozen area only for a concrete correctness counterexample, proof/authority seam, measured complexity/performance regression, new R&D requirement, or public API/DX requirement.

The active project phase is now **productization through a public Rust application facade over the universal runtime boundary**. The ordinary adaptive transaction path now also carries `kernel-change` rebase certificates through durable residual publication for both relation-only and mixed/object commits, with stable client intent separated from the realized history/recovery effect. Pass281 introduced `cfmd-runtime`; Pass337 adds the user-facing `cfmd` crate; the product line now includes object-first typed Rust DX, Plan/Candidate preview, durable history with exact mixed undo/redo, historical worlds through `db.at(revision)`, certified non-head undo/rebase, exact query-result watch, provider-neutral wake delivery, deterministic watch lifecycle/catch-up, and P299 hosted session/permission authority, P300 transport-neutral hosted protocol ingress, P301 exact hosted watch subscriptions, and P302 canonical transport-neutral wire framing/negotiation, P303 hosted server composition, P304 dynamic hosted security lifecycle, and P313 unified database construction/opening with parity-complete single-file storage as the default new-store path. Rust applications depend on `cfmd`; Python/.NET/Studio bind to the stable runtime protocol. None call `kernel-*` crates directly.


### PASS507 checkpoint

The current product mainline is PASS512. Normal mutable Rust application ownership is `Context<M>`; cross-schema stale publication uses one kernel-owned `PreparedSchemaAwarePublication`. Formation-world `require`/relational observations are sealed only at the first migration boundary. PASS512 replaces per-intent global causal-history replay for relational formation seals with a persistent relation-delta lineage retained in the schema epoch, and exposes stable `FormationProofUnavailable` / `FormationProofInvalidated` diagnostics distinct from current-world transaction conflict.

## Product direction

The implementation order is Rust-first. Rust applications depend on the public `cfmd` crate; `cfmd-runtime` remains the universal runtime/binding anti-corruption layer below it. The normal Rust path is object-first:

```rust
use cfmd::prelude::*;

let db = Database::builder("app.cfmd")
    .create_authoritative::<AppSchema>()?;
let ctx = db.context::<AppSchema>()?;
let todos = ctx.todos.all()?;
```

Object-first Rust DX uses stable textual domain keys and symbolic field/reference accessors. The relation/value/query IR remains available explicitly through `cfmd::dynamic` for generated bindings, tooling and genuinely dynamic applications; it is not the default application vocabulary.

Typed/generated Rust DX will be layered over this stable IR rather than replacing it. The intended higher-level application experience remains familiar lazy querying over deep domain paths, with CFMD-specific capabilities around the query:

```python
users = db.users.where(
    lambda u: u.active & (u.passport.country.code == "RU")
)

async for delta in users.watch():
    ...

future = db.preview(plan)
future.delta(users)
future.why_changed(users)
future.commit()
```

The facade rules are stricter than a conventional ORM:

- no hidden storage I/O from ordinary materialized Python attribute access;
- deep relationship traversal is symbolic inside query construction;
- many-valued paths require explicit `any/all/match/aggregate` semantics;
- current, historical and speculative candidate worlds are explicit;
- exact watch is a runtime protocol, not callback-driven polling;
- Plans/Candidates/history use the same authoritative transition pipeline;
- one authoritative runtime owns writes; external Studio/tooling attaches to it rather than editing files independently.

Public Rust API compatibility/provenance policy: [`docs/api/PUBLIC_RUST_API_COMPATIBILITY.md`](docs/api/PUBLIC_RUST_API_COMPATIBILITY.md).

See [`docs/api/PRODUCT_ROADMAP.md`](docs/api/PRODUCT_ROADMAP.md) and the retained design source [`docs/api/CFMD_PYTHON_FACADE_THEORY.md`](docs/api/CFMD_PYTHON_FACADE_THEORY.md).

## Documentation map

- [`SPEC.md`](SPEC.md) — concise repository-facing specification;
- [`docs/spec/CFMD_CORE_SPEC.md`](docs/spec/CFMD_CORE_SPEC.md) — full normative core specification and append-only implementation record;
- [`docs/status/PROJECT_STATUS.md`](docs/status/PROJECT_STATUS.md) — current phase and release boundary;
- [`docs/status/KERNEL_HOSTILE_LEDGER.md`](docs/status/KERNEL_HOSTILE_LEDGER.md) — current kernel audit/freeze inventory;
- [`docs/status/PRODUCTIZATION_LEDGER.md`](docs/status/PRODUCTIZATION_LEDGER.md) — active public-product closure ledger;
- [`docs/architecture/ARCHITECTURE.md`](docs/architecture/ARCHITECTURE.md) — current runtime/product layering;
- [`docs/architecture/CRATE_MAP.md`](docs/architecture/CRATE_MAP.md) — internal workspace crate graph;
- [`docs/api/RUST_API_ROADMAP.md`](docs/api/RUST_API_ROADMAP.md) — primary Rust product/runtime roadmap;
- [`docs/api/PRODUCT_ROADMAP.md`](docs/api/PRODUCT_ROADMAP.md) — cross-language product sequence;
- [`docs/reports/current/PASS280_REPORT.md`](docs/reports/current/PASS280_REPORT.md) — kernel-refactor closeout report;
- [`docs/reports/current/RELEASE_PREP_2026-09-28.md`](docs/reports/current/RELEASE_PREP_2026-09-28.md) — repository/CI/documentation release-prep report;
- [`docs/status/SUPPORT_MATRIX.md`](docs/status/SUPPORT_MATRIX.md) — certified durability scope;
- [`formal/lean/README.md`](formal/lean/README.md) — Lean proof artifacts and refinement gates.

## Toolchains

- Rust: **1.98.1**, edition 2024.
- Lean: **4.34.0**, core-only proof artifacts (no Mathlib dependency).
- Third-party Rust dependencies are vendored under `vendor/` for reproducible/offline builds.

## Quick verification

Repository completeness first:

```bash
bash ./scripts/verify-repository.sh
```

Rust gates:

```bash
bash ./scripts/ci-rust.sh
```

Formal/refinement gates:

```bash
bash ./scripts/ci-formal.sh
```

The repository verifier checks required project files, the repository manifest, and literal Rust `include!("...")` targets. A partial upload such as `physical_store.rs` without `physical_store/core.rs` therefore fails before Cargo compilation.

## Layout

```text
crates/                 34 Rust workspace crates (28 internal kernels/infrastructure + public `cfmd`/`cfmd-derive` SDK + runtime/protocol/host/transport stack)
formal/lean/            Lean proofs + source-refinement binders
vendor/                 vendored Rust dependency closure
artifacts/              retained diagnostics/evidence/certification records
docs/spec/              normative specification
docs/architecture/      current architecture documentation
docs/status/            project status, support and hostile ledger
docs/api/               product/facade design and roadmaps
docs/reports/current/   current closeout/release-prep reports
docs/reports/archive/   historical pass reports
docs/history/           older provenance/spec snapshots/manifests
.github/workflows/      Rust and Lean CI
scripts/                local verification/statistics helpers
```

## Development policy

1. Logical/semantic contracts are authoritative; physical optimizations must not silently redefine them.
2. Exact semantic equality/order is defined by pinned `Γ`, not incidental Rust `Eq`/`Hash`/`Ord` where semantic modules apply.
3. Recoverable failure must occur before authoritative publication/commit boundaries.
4. Durability claims are scoped to certified storage profiles; unsupported profiles fail closed.
5. Public facades must not expose internal crate ownership/layout merely because those types exist in Rust.
6. Exact watch/Candidate/history/tooling must share the same revision/change semantics rather than grow parallel event systems.
7. New proof-boundary vocabulary or durability fault points must update the corresponding formal/refinement gate.
8. Frozen kernels reopen only on evidence, not cleanup-by-inertia.

## License

Workspace crates declare `MIT OR Apache-2.0`. See `LICENSE-MIT` and `LICENSE-APACHE`.

## Object-first entity references (Pass285)

The Rust product layer distinguishes value objects from identity-bearing entities. The normal P338 surface is `#[derive(CfmdEntity)]` with a stable `#[cfmd(key = "...")]` and explicit typed `#[cfmd(id)] Id<T>`; the older `cfmd_entity!` macro remains a transparent compatibility/advanced declaration path. P375 also makes database-owned field rules part of the entity descriptor itself: `#[cfmd(range(...))]`, `#[cfmd(length(...))]`, and `#[cfmd(one_of(...))]` lower into the same schema/validation authority used by every binding and by recovery. `Ref<T>` fields generate symbolic deep-path accessors. Materialized references are inert Rust values and never perform hidden I/O; traversal lowers to the existing relation/Γ query IR. Strong references are validated against the final composed `Plan` state, and entity identities are unique within their object relation. P380 makes authoritative entities strictly current and legacy-free. Persisted schema evolution is expressed by an explicit kernel/runtime migration model rather than `rename_from` metadata. A non-authoritative Context contract may keep any local spelling with `#[cfmd(bind = "persisted_name")]`; that binding is client-local and does not alter persisted schema authority.

```rust
#[derive(CfmdEntity)]
#[cfmd(key = "app.user")]
struct User {
    #[cfmd(id)]
    id: Id<User>,

    #[cfmd(length(min = 3, max = 64))]
    name: String,

    #[cfmd(range(min = 0, max = 150))]
    age: i64,

    #[cfmd(one_of("user", "admin"))]
    role: String,
}
```

Rule bounds and membership values are literal schema data, not arbitrary Rust expressions or callbacks. The derive rejects incompatible rule/type combinations at compile time.

```rust
let people = snapshot.objects::<Person>()?;
let russian = people.where_(|person| person.passport().country().code().eq("RU".to_owned()));
```

`Ref<T>` is required-one, `Option<Ref<T>>` is optional-one, and `Many<T>` is a first-class zero-to-many object relationship value. The object declaration is the public schema authority: `children: Many<Child>` does not require `Child` to carry an artificial backlink. `cfmd-runtime` lowers that object relationship into an internal typed edge relation used by query/Γ execution and lifecycle normalization.

### Rust object cardinality and materialization (P342)

Materialized objects keep `Ref<T>` / `Many<T>` as real Rust fields, but field access itself never performs I/O. A materialized relationship is snapshot-bound and exposes explicit object-first operations: `ref.load()`, `many.all()`, `many.where_(...)`, `many.count()`, `many.one()`, and the lower-level composable `query()`. Normal application code does not name joins, foreign keys, `include`, or internal edge relations.

Relationship selections stay first-class. `many.where_(...)` can `detach_all()` or `move_to(...)` directly; these operations project target identities and mutate edge facts without constructing target Rust objects. `move_ids_to(...)` is fail-closed when an ID is not attached to the source owner. `CandidatePreview::derived()` separates policy/lifecycle consequences (for example orphan deletion) from explicit Plan row mutations.


Ordered object predicates are also Γ-native rather than Rust/SQL comparisons. Canonically ordered fields expose `greater_than`, `greater_than_or_equal`, `less_than`, `less_than_or_equal`, and inclusive `between`; object schema generation installs a stable per-field ordering identity and the matching semantic ordering module. The relational IR carries `FilterOrderConst`, query preparation pins/compiles that ordering, and maintained watches filter exact deltas directly. Optional/structural fields receive no ordering API until an explicit structural ordering contract exists.

Query composition stays on the same exact relational calculus. Predicates can be conjoined with `ObjectPredicate::and`, disjoined with `ObjectPredicate::or`, and negated with `ObjectPredicate::not`. `or` is a first-class Γ-aware Union rather than a callback or a De Morgan expansion through multiple Difference nodes: Set queries union support classes, while Bag queries add multiplicities. Canonically ordered fields can then select exact Γ-order boundaries without materializing and sorting Rust objects:

```rust
use cfmd::prelude::*;

let selected = children
    .where_(|child| {
        child
            .score()
            .greater_than(3)
            .and(child.score().less_than_or_equal(10))
    })
    .top(5, |child| child.score());
```

`top` and `bottom` expose the existing maintained `TopKWithTies` semantics. Boundary-equivalent rows are retained, so the result may contain more than `k` rows; this is selection by the pinned Γ ordering, not a promise of physical result-row ordering. Exact watches use the same maintained ordered-boundary state.

Object-first `select` preserves source multiplicity by lowering through the kernel-native bag law before projection. If four distinct objects project to `3, 7, 7, 11`, ordinary `select` returns four projected values. Semantic quotienting is explicit and still kernel-native:

```rust
let scores = children.select(|child| child.score());
assert_eq!(scores.count()?, 4);

let unique_scores = children.select(|child| child.score()).distinct();
assert_eq!(unique_scores.count()?, 3);
```

This is not host-side bag emulation: the lowering is `Set<Object> -> PromoteToBag -> Project`, and `.distinct()` is the existing maintained `Distinct` operator using the projection's declared Γ equivalences. Exact watches preserve the same distinction: a duplicate source projection emits one bag insertion for ordinary `select`, while an explicit distinct watch suppresses it.

Grouped aggregation also stays inside the same kernel calculus:

```rust
let counts = children.group_by(|child| child.score()).count();
let regional_counts = users
    .group_by(|user| (user.team(), user.region()))
    .count();
let sums = metrics.group_by(|metric| metric.bucket()).sum(|metric| metric.value());

let current_counts = counts.all()?;
let mut count_watch = counts.watch()?;

let busiest = metrics
    .group_by(|metric| metric.bucket())
    .count()
    .top(10);
```

`count()` and `sum(...)` build typed aggregate queries rather than immediately collapsing into a `Vec`. Exact reads and maintained watches therefore interpret the same kernel `Group` expression. `group_by` accepts either one field or a tuple of fields; tuple keys lower to the kernel's native `group_columns + group_equivalences` product key, so composite grouping is not implemented by host-side maps or arity-specific engines. `count()` uses kernel `ExactCount`; floating `sum(...)` uses kernel `ExactF64Sum`. Aggregate queries can continue into `top(k)` / `bottom(k)`, which order by the aggregate value after the complete key and preserve all Γ-equal boundary groups. This lowers directly to `Group -> TopKWithTies`, including Candidate and maintained-watch execution; there is no host-side sorting or exact-k truncation. Candidate reads expose the same typed aggregate result shape. No `group_by2`, `watch_count` / `watch_sum`, or host-side regrouping API exists.

Detached construction is graph-first. `Many::new([Child { ... }, ...])` is accepted by generated `cfmd_new(...)`; insertion lowers the complete object graph into one `Plan` containing entity rows and internal edge facts. On update, a bound `Many<T>` returned unchanged means preserve that relationship, while assigning `Many::new(...)` / `Many::empty()` replaces it atomically. Endpoint deletion is lifecycle-safe: internal edge rows carry live endpoint witnesses, so kernel revision normalization removes dangling edge facts without SDK cascade scans.

### Kernel-backed entity lifecycle (P287)

Object-first `Id<T>` remains a type-local external identity, while the runtime deterministically maps `(TypeId, Id<T>)` into a kernel lifecycle `EntityId`. Object relations stay the canonical query/storage shape; `cfmd-runtime` derives kernel carriers, lifecycle roots and mirrored `LiveEntityRef` fields for reference columns before building the target Revision.

Strong-reference integrity is therefore checked by kernel model normalization and survives durable reopen. Deleting a referenced target is rejected even when the current Plan contains only the target object's contract. Relation-first plans remain on the compact relation-only path; entity plans currently use the correctness-first full revision publication path while lifecycle/carrier state changes.

### Incremental mixed semantic publication (P288)

Entity Plans no longer force a full physical-store rebuild. `kernel-plan` now exposes a mixed revision transition: the exact target Revision remains the logical/lifecycle authority, while declared relation deltas update physical relations and maintained materializations incrementally. Durable recovery still records the exact full target revision bytes, so restart/idempotency semantics remain unchanged. Compact encoding of the non-relation lifecycle/carrier/field delta is a separate durability-format optimization, not a correctness dependency.

### Candidate diagnostics and typed preview (P291)

Candidate now exposes a typed preview surface over the same exact proposed Revision used by commit. Object candidate queries support typed `select(...)`; `effects()` summarizes touched relations and row/lifecycle effects; `diagnostics()` reports certified-target validity plus live-runtime readiness (`Ready`, `Stale`, or `RuntimeClosed`); and `preview()` packages source/target revision, effects and diagnostics into one immutable product summary. Preview remains readable after the owning Database closes because Candidate owns the certified proposed Revision rather than a live runtime state.

### Durable history + exact mixed inverse (P292–P293)

`Database::history()` projects the authoritative durable causal effect ledger into product-facing `HistoryEntry` values with transaction identity, source/target revisions, causal prerequisites, exact relation changes and explicit reversibility. Ordinary application code feeds history back into the same visible transaction language as every other write: `db.undo(&mut tx, entry)` or `db.undo_latest(&mut tx)`, followed by `db.preview(&tx)` / `db.commit(&tx)`. The exact inverse is still an ordinary internal `Plan`; `HistoryEntry::undo_plan()` remains the explicit low-level/tooling escape hatch rather than a required application concept.

P293 closes the mixed-object inversion gap for newly written durable transactions. Every mixed commit now persists the compact exact reverse `DurableModelDelta` beside its forward delta in WAL/idempotency authority (codec v11). Entity creation/deletion, lifecycle/carrier changes and reference-field rewrites therefore remain exactly undoable after close/reopen without storing a second full Revision and without a relation-only fallback. Older v10 mixed records remain readable and are still classified `ComplementRequired` when they lack that historical complement.

### Historical worlds with `db.at()` (P294)

`Database::at(revision)` reconstructs one exact immutable committed world from the current durable head plus the existing causal effect authority. It does not persist per-revision snapshots and does not approximate across an unavailable/non-reversible history boundary. The returned `ReadContext` reuses the ordinary schema, relation and typed object query vocabulary but deliberately carries no write capability.

History is revision-anchored: `db.history()` is history at the current HEAD, while `db.at(r)?.history()` is the causal ideal visible from `r`. Reconstruction follows certified reversible edges in the causal effect graph, so exact branch revisions are eligible when they are connected by retained reversible effects. Mixed lifecycle/reference history uses the P293 forward/reverse `DurableModelDelta` pair.

### Exact query-result watch (P296)

The Rust product layer now exposes revision-tagged exact query-result subscriptions without an async-runtime dependency. `ObjectQuery::watch()`, projected object queries, and `ReadContext::watch(&Query)` compile the existing kernel `MaterializedRelPlanState` once at subscription creation and then advance it only with committed relation deltas from durable causal history. Per-revision events contain exact inserted/removed result rows; no old/new full-query replay or hidden recompute fallback is used.

```rust
let snapshot = db.snapshot()?;
let mut watch = snapshot
    .objects::<Country>()?
    .where_(|country| country.id().eq(id))
    .select(CountryFields::code)
    .watch()?;

let initial = watch.initial();
let delta = watch.recv()?;
```

`recv()` blocks on an in-process kernel publication signal implemented with `std::sync::Condvar`; `try_recv()` is non-blocking. The signal is wake-only, not state authority: after wake the subscription reads the committed causal effect and feeds its exact relation delta through the maintained query differential program. Historical `db.at(...)` views are immutable and cannot create a live watch. P296 deliberately does not claim cross-process wake-up yet; that requires a later transport/OS-notification layer rather than polling the database files.

### Host-provided publication notification (P297)

Watch wake-up is now a provider boundary rather than a hard-wired IPC or `Condvar` architecture. The ordinary embedded path still uses a first-party in-process notifier, while a host can explicitly supply `PublicationNotifier` to `Database::create_with_publication_notifier` / `open_with_publication_notifier`. The provider carries no database state and has no write authority: after every wake, exact watch still reads the durable causal transition and advances the maintained query state from that authority.

This is also the hosted-CFMD security boundary: database open/create never starts an IPC/TCP listener or implicitly trusts local peers. Future local/network transports, authentication and authorization compose above the runtime as explicit providers/services; semantic writer conflict/rebase correctness remains inside the kernel.

### Watch lifecycle and bounded catch-up (P298)

Exact watch now has an explicit lifecycle contract. Every watch exposes a cloneable `WatchCancellation`; `close()`/`cancel()` wake a blocked `recv()` without timeout polling and subsequent receive attempts fail with `ErrorKind::WatchClosed`. The kernel wait handle owns only the notifier, not `DurableRuntime`, so a blocked receiver cannot keep the database runtime alive; final runtime drop emits a wake-only shutdown signal and the waiter terminates deterministically.

`WatchStatus` distinguishes `Current`, `Lagging`, `Cancelled`, `RuntimeClosed` and `Unavailable`. Lag is measured as the exact count of committed causal transitions between the watch anchor and current HEAD. A watcher has no in-memory event backlog: it stores one maintained query state plus its anchor and consumes at most one durable `HistoryEffect` per receive. Catch-up is therefore sequential and bounded by the durable history authority rather than by an unbounded RAM queue; if exact causal coverage is unavailable, watch fails closed instead of dropping transitions or recomputing the whole query.

P345 freezes the runtime-neutral readiness/drain boundary. P346/P347 establish race-free standard-library `Waker` registration with no polling, helper thread, executor-owned cursor, or mandatory Tokio dependency. P348 removes the transitional `cfmd-async` wrapper entirely: raw/object/projection watches are directly awaitable through `watch.next().await`, while the same object still exposes blocking `recv()`, nonblocking `try_recv()`, and bounded `drain_ready(max_events)`. Dependency-frontier readiness indexes pending wakers by exact scanned relations, so a relation publication wakes only subscriptions whose maintained dependency frontier intersects the effect; opaque/full/schema signals still broadcast. Output-equivalent causal transitions are quotiented from the public stream instead of producing empty events. Durable causal history and maintained query state remain the sole event authority; Tokio 1.53.1 remains dev-only compatibility coverage.

### Hosted session/security authority (P299)

Hosted composition now has a transport-independent security boundary. A trusted authentication/hosting layer maps an external identity to `PrincipalId` and constructs a `Session` with explicit `PermissionSet` grants. `Database::session(session)` returns `SessionDatabase`; permissions are then propagated through product values rather than checked only at endpoints.

Authorization is now semantic-coordinate based. Coarse `Read`/`Write` remain explicit whole-database grants, while relation/field/object/relationship grants authorize exact query/change footprints. `ModelRead` separately controls full authoritative schema inspection and `SchemaMigrate` separately controls schema publication; generic data read/write does not imply either. Named `Role` values are immutable DX bundles that flatten into the same `PermissionSet` law before a session is constructed—role names, hierarchy, and frontend policy do not participate in runtime enforcement. `ReadContext::schema_revision()` exposes only the schema epoch needed for compatibility decisions; full `schema()` metadata requires `ModelRead`. Embedded `Database` remains an explicit unrestricted local capability; hosted adapters should expose `SessionDatabase`, not raw `Database`.



### Transport-neutral hosted protocol boundary (P300)

`cfmd-protocol` is now a separate product crate above `cfmd-runtime`. It owns protocol-facing values, rows, query AST, history DTOs, relation mutations, requests/responses and stable protocol error codes; public protocol signatures do not expose kernel types, `Plan`, `Query`, `Row` or other Rust-runtime implementation values. `HostedSession` is constructed only from an already-restricted `SessionDatabase`, so transport code never needs raw `Database`.

The first unary protocol surface covers current revision, current/historical query, revision-anchored history and relation commit. Commit requests carry an explicit `base_revision` and transaction id; stale bases fail closed before publication while ordinary runtime transaction/idempotency authority remains unchanged. Protocol resource limits bound query AST nodes/depth, mutation count, rows, row width and recursive value complexity before runtime execution. Internal/recovery/invariant errors are sanitized at the protocol boundary rather than forwarding implementation diagnostics to an untrusted client.

P300 deliberately added no listener, codec, TCP/IPC or authentication mechanism. P301 now adds transport-neutral watch stream semantics inside the same protocol layer; concrete framing and transport remain adapters above it.

### Exact hosted watch subscriptions (P301)

`cfmd-protocol` protocol version 2 adds server-side exact watch subscriptions without introducing a protocol event log or polling loop. `OpenWatch` validates the protocol query and constructs the ordinary restricted-runtime `QueryWatch`; its response returns a protocol-owned `SubscriptionId` plus the exact initial result/revision. `NextWatch` is a blocking pull over `QueryWatch::recv()`. Output-equivalent causal transitions are consumed internally but quotiented from the public stream, so the next observable event may span several durable revisions while still carrying exact source/target revisions and delta.

`WatchStatus`, `CancelWatch`, and `CloseWatch` expose the P298 lifecycle without granting write authority. Cancellation is stored separately from the maintained query-state lock, so a transport can cancel a currently blocked consumer. A session-level watch-count limit bounds maintained subscription state, and only one consumer may hold a subscription at a time. `CloseSession` cancels every outstanding subscription so a future transport disconnect can deterministically release blocked server work. None of these protocol lifecycle operations can mutate authoritative Revision; Lean extends the wake-only theorem to protocol open/next/close/session-close.

P301 still defines no wire codec, listener or network/local IPC stack. A transport may map blocking `NextWatch` onto a push frame, async stream or request/response mechanism, but it must not invent event authority or bypass the session/runtime permission boundary.


### Canonical hosted wire framing and negotiation (P302)

`cfmd-protocol::wire` now provides a transport-neutral canonical binary representation for the P300/P301 hosted semantics. The wire contract has a fixed 20-byte header (`CFMD` magic, wire framing version, frame kind, flags, request id and payload length), canonical big-endian scalar encoding, explicit variant tags, bounded collection/string/value decoding and exact request-id correlation. A transport can call `decode_header()` on the fixed header before allocating or reading the declared payload; oversized frames therefore fail at admission rather than after a large allocation.

Wire framing version **1** negotiates hosted protocol version **2** through `ProtocolHello` / `ProtocolHelloAck` and intersects explicit capability bits for query, history, commit and watch. A `WireHostedSession` refuses ordinary request frames until negotiation succeeds and dispatches decoded requests only through the existing restricted `HostedSession`; semantic errors are returned as sanitized protocol error DTOs. Unknown frame kinds, protocol variants, non-canonical booleans/options, duplicate product fields, trailing bytes and incompatible versions fail closed. The codec deliberately does not implement permissive unknown-field skipping: DTO evolution is version/capability negotiated instead.

P302 still owns no listener, socket, TLS/authenticator, local IPC implementation or network server lifecycle. TCP, IPC, QUIC and Studio/CLI links remain adapters that move already-framed bytes and preserve the session/authentication boundary.

### Hosted server composition boundary (P303)

`cfmd-host` composes the existing restricted runtime/session protocol into a transport-neutral hosted server boundary. Concrete transports do not receive raw `Database`: an `Authenticator<Evidence>` establishes only `PrincipalId`, an independent `Authorizer` returns `PermissionSet`, and the host alone constructs `Session -> SessionDatabase -> HostedSession -> WireHostedSession`.

The host owns connection admission and bounded concurrent frame execution, but no listener, socket, TLS stack, IPC protocol or background thread. `HostedConnection::close()` and server shutdown close the underlying hosted session, cancelling blocked watch consumers. Localhost/process/transport metadata is never an implicit grant source; transport providers must supply evidence to the configured authentication authority.

### Dynamic hosted security lifecycle (P304)

Hosted sessions no longer freeze a copied `PermissionSet`. `Session` owns one shared authority cell, and every restricted product value derived from it observes the same current generation. Host authorization refresh can therefore downgrade or upgrade future authority without rebuilding the database/session stack; existing `Plan`, `Candidate`, history inverse and query/watch values cannot retain stale write grants. Revocation is monotone: once revoked, the session cannot be revived by permission refresh and all subsequent authority checks fail closed.

`cfmd-host` now exposes explicit `ChannelBinding::{Unbound, Bound}` authentication context, expiring `AuthorizationGrant`s, `next_expiration()` / `expire_due(now)` scheduling hooks, provider-driven authorization refresh/revocation, and graceful `drain()`. The host still owns no timer thread or transport runtime: a future TCP/IPC event loop may schedule the nearest expiry exactly, while `expire_due` closes affected hosted sessions and wakes blocked watch consumers. Draining rejects new connections but leaves already-admitted work under ordinary CFMD transaction semantics until those connections close. Locality remains non-authoritative.


### First-party local IPC transport conformance (P305)

`cfmd-transport-local` is the first concrete transport adapter above `cfmd-host`. On Unix it exposes a Unix-domain-socket provider and a small sequential client helper. The endpoint is created with `0600` permissions, but filesystem locality is not identity: peers still send bounded authentication evidence, which is forwarded unchanged to the configured host `Authenticator`; authorization remains owned by the configured `Authorizer`.

The transport uses the existing P302 wire frames and P303/P304 hosted lifecycle. It has a dedicated reader path plus a bounded fixed worker set and bounded response backlog. A blocked hosted `NextWatch` therefore does not prevent the transport from observing socket EOF: disconnect closes the hosted connection, revokes/cancels the session and wakes the blocked watch without polling. P305 intentionally does not add implicit peer trust, a second permission model, or transport-owned writer authority. The first implementation is Unix-only; a Windows named-pipe implementation can conform to the same host/wire contracts without changing CFMD semantics.

### Single-file `*.cfmd` physical foundation (P306)

`kernel-durability` now contains the first self-contained single-file generation container. A database generation is appended inside one file, fully synchronized, and only then made authoritative by alternating checksummed root slots on separate 4 KiB pages. Recovery can ignore an unpublished appended generation or a torn replacement slot, but never silently falls back when a checksum-valid published root references corrupt generation bytes. Generation and section SHA-256 digests, parent-digest chaining, range validation, and streaming section I/O make the container suitable as the physical basis for a future one-file product backend without temp/sidecar recovery authority.

P306 deliberately does not claim that the existing live WAL/checkpoint store has already been migrated: the next storage pass must bind the current WAL/recovery/checkpoint machinery to this container while preserving its existing crash/freshness laws.

### Single-file live WAL authority (P307)

The single-file container now hosts the real CFMD WAL directly inside `*.cfmd`. It reuses the existing WAL frame/prepare/commit/recovery implementation as an offset-bounded region rather than defining a second transaction log. Root records persist the active journal start, exact first LSN, and—during generation rotation—a sealed byte boundary plus exact next LSN. Rotation follows `WAL durable -> sealed root durable -> generation durable -> new root durable`; therefore a crash while building the next generation falls back to a precisely sealed old WAL without magic scanning or a sidecar file. Recovery can truncate a torn active WAL only at the existing scanner's last-good frame boundary and can reopen a sealed old journal after discarding unpublished generation bytes.

P307 still does not switch the complete `DurableRevisionStore`/product `Database` to a single-file backend. The remaining storage integration is to map checkpoint/metadata/prepared/replication/physical-artifact publication and streaming-checkpoint rotation onto these container sections while preserving the current store's authority/freshness laws.

### Single-file DurableRevisionStore path (P308)

`DurableRevisionStore` now has a real one-file physical path through `create_single_file(...)` and `open_single_file(...)`. Ordinary prepare/commit/group-commit semantics continue to use the same `PreparedCommitAuthority` and production `FileRevisionWal`; the active WAL is the P307 region inside the `*.cfmd` file. Synchronous checkpoint rotation serializes the checkpoint, durable metadata, and prepared-cut capsule as checksummed generation sections, seals the current WAL, publishes the next generation, reopens the carried-LSN journal, and preserves unresolved prepare identity across the cut. A create -> commit -> reopen -> rotate -> commit -> reopen regression verifies that only the single `database.cfmd` path exists.

At P308 this path was intentionally not yet advertised as full parity with the directory backend: replication authority, streaming checkpoint, external freshness, and physical compaction were still fail-closed rather than redirected to hidden sidecars. P309, P311, and P312 subsequently close those physical parity gaps while preserving the same one-file authority law.

### Single-file replication authority lane (P309)

Single-file `DurableRevisionStore` persists replication authority without `replication.cfre`. Live replication records use the existing replication codec inside the active WAL and share its physical durability barrier, so replication-only writes never advance the linear database head. Checkpoint rotation writes only the frozen live authority delta as a content/parent-bound `CFAS` segment carried by an authenticated immutable `CFAO` object and a fixed `CFLN` locator; an empty delta reuses the current root. Recovery authenticates and replays exactly the root-reachable segment chain, then the live WAL suffix. Physical compaction copies authenticated object bytes exactly and rebuilds only relocation locators. The old generation `ReplicationAuthority` archive section is absent from the active format and there is no monolithic fallback path.

At P309 pinned-cut streaming checkpoint remained deliberately fail-closed rather than weakening ordinary LSN-contiguous root transitions or buffering an unbounded shadow WAL in RAM. P311 closes that payer with a separately certified overlapping/carry-forward transition whose copied suffix is rescanned to the exact durable endpoint before root publication.

### Physical durability backend separation (P310)

`DurableRevisionStore` no longer models the directory layout plus an optional single-file escape hatch. Physical ownership is represented by one `DurabilityBackend`: `Directory` owns the directory root and lock, while `SingleFile` owns the `SingleFileContainer`. The store's transaction/revision/idempotency/causal state remains independent of that choice. Backend capabilities now state whether streaming checkpoint, external freshness, and physical compaction are available; unsupported operations are rejected before mutating store state. Captured replication-authority frames are also handed to the backend for physical persistence rather than branching on layout in the replication facade.

P310 deliberately keeps the existing directory streaming-checkpoint implementation unchanged while establishing the boundary required to implement single-file carry-forward streaming next. The remaining generation-publication specialization is localized behind the backend owner rather than represented by `directory + Option<SingleFileContainer>` state in the semantic store.

### Single-file streaming checkpoints (P311)
The single-file backend now supports the same pinned-cut streaming semantics as the durability state machine without creating a shadow sidecar. Commits may continue while a checkpoint is prepared; publication carries the exact WAL suffix from the pinned cut into the new generation and proves that `checkpoint + suffix` recovers the current durable head before the root switch.
### Single-file freshness and compaction parity (P312)

External freshness is now a backend-neutral durability contract instead of a directory-layout calculation. Both backends provide one `FreshnessRecoveryMaterial`: immutable generation binding/digest plus the exact bounded WAL region and first LSN. Single-file freshness therefore covers ordinary commits, synchronous rotation, P311 carry-forward streaming, and reopen without inspecting bytes beyond the root-authoritative journal boundary. A freshness-anchored single-file database must use the freshness-aware open path; ordinary open fails closed. Rollback/truncation remains detectable after physical compaction because relocation preserves the generation/WAL freshness material.

Single-file physical compaction is also first-class. The backend relocates the current authoritative generation plus sealed live WAL into the reclaimable front region of the same `*.cfmd`, verifies generation and WAL digests/recovery at the destination, then publishes a certified same-generation relocation root. Only after the new root is durable is the obsolete tail truncated and the live WAL reopened on an independent file handle. Compaction therefore changes physical offsets/root sequence only: logical generation, Revision, authority digests, freshness binding, prepared state, replication state, and WAL endpoint are preserved. No temp file, sidecar, semantic checkpoint, or fallback backend participates.


## Pass313 — unified database product construction

`DatabaseBuilder` is now the canonical product entry point. `Storage::Auto` resolves an existing file as single-file, an existing directory as directory-backed, and a new path as single-file; `Storage::Directory`/`SingleFile` remain explicit overrides. `Database::create/open` are thin sugar over this same builder path rather than separate construction semantics. Publication notification is a builder concern because it belongs to the opened runtime, while authentication/authorization remain hosted-service concerns. `cfmd-host` therefore exposes `DatabaseHostingExt`, allowing `db.host(authenticator, authorizer)` without adding a dependency from `cfmd-runtime` back to hosting.

## Scoped Context unit-of-work control (Pass495–502)

Normal Rust mutation has one mutable owner: a bounded `Context<M>`. Admission pins one formation world, staged effects advance only the Context Candidate, application-visible reads become exact causal observations, and publication is explicit. Dropping an uncommitted Context discards the unit of work.

```rust
let db = Database::builder("app.cfmd")
    .create_authoritative::<AppSchema>()?;
let ctx = db.context::<AppSchema>()?;

ctx.add(|schema| &schema.todos, first)?;
ctx.add(|schema| &schema.todos, second)?;

let preview = ctx.preview()?;
let outcome = ctx.commit()?;
```

Reference and relationship writes use the same owner; no second public transaction container is required:

```rust
ctx.attach(&owner.assets, asset_id)?;
ctx.move_to(&owner.assets, asset_id, &other.assets)?;
ctx.undo(history_entry)?;
```

`Snapshot<M>::edit()` creates the semantically distinct strict snapshot-bound Context: if HEAD advances, publication fails stale rather than transporting silently. Internally the runtime uses one `IntentJournal` implementation for exact effects, retry identity, requirements and Candidate publication; it is hidden implementation/binding plumbing rather than normal application DX. `TransactionId` remains a genuine durable retry/protocol identity.

Authoritative typed creation consumes `S: DatabaseDefinition` only at creation time through `DatabaseBuilder::create_authoritative::<S>()`; the returned object is still schema-neutral `Database`. Dynamic/generated bindings may continue to supply raw `Schema` through the runtime builder without creating a `Database<S>` wrapper.

### Semantic transaction transport and durable retry identity (Pass355–357)

`Database::transaction_readiness(&tx)` distinguishes `Ready`, `Rebasable`, and `Conflict`. `Database::preview(&tx)` and, from Pass357, ordinary `Database::commit(&tx)` may transport an exact proposed effect to current HEAD only when runtime causal history proves compatibility through Γ-canonical coordinates and action laws. Opaque, coordination-required or conflicting history fails closed.

Pass357 also separates the *client semantic intent* from one concrete durable realization. Relation/mixed intents retain exact source/target lineage for recovery, but retry identity compares the canonical forward effect and pinned semantic authority. Therefore a transaction certified from an older source onto a newer HEAD can be retried after an uncertain response and still returns `AlreadyCommitted` for the actual published revision instead of becoming a different transaction merely because its realization revisions changed. No second retry ledger or intent hash was introduced: WAL/checkpoint already retain the canonical durable intent needed for this comparison.

### Action-law-aware transaction transport (Pass356)

Pass356 strengthens the P355 runtime bridge: exact causal footprints now retain `RewriteActionLaw` instead of collapsing every write to coordinate presence. Forward transaction transport therefore uses the same kernel-change action classifier as the rewrite layer: disjoint actions strongly commute, equal set-presence intents can be idempotent, opposite presence intents are definite conflicts, and unresolved overlapping laws require coordination. Set relation classes expose presence laws; bag classes remain opaque until exact multiplicity action semantics are carried. Object carrier membership also remains opaque so two writes to one entity identity cannot be incorrectly declared compatible merely because lifecycle presence agrees. Historical undo intentionally keeps its stricter overlap rule because undoing an earlier idempotent action must not erase a later independent intent.

### Unified mutation/history DX + ordered hostile closure (Pass359–360)

Pass359–360 established the shared Plan/Candidate/kernel semantics later internalized by scoped `Context<M>`. The old public `&mut Transaction` accumulation shape is superseded by the Context-owned `IntentJournal`; undo-of-undo remains redo and no separate redo engine exists.

The Γ-native `F64Total` object predicate path is hostile-checked at the public exact-query and maintained-watch surfaces for `NaN`, signed zero, and infinities. The ordering remains the pinned total-order semantic module; no Rust partial-order fallback, post-materialization filter, or watch recomputation path is introduced.
