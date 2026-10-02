# CFMD Product Roadmap

## Product architecture

CFMD productization is **Rust-runtime first, language-surface second**.

The desired Python/DX theory remains valid, but its semantics are implemented first as a universal Rust product boundary:

```text
Rust applications      Python / .NET / Studio
       |                     /
      cfmd                  /
       \                   /
             cfmd-runtime
                  |
       query / plan / candidate / watch / history
                  |
             kernel-* graph
```

No language binding may call the kernel graph directly. This avoids duplicating semantics, leaking internal ownership types, and coupling FFI compatibility to kernel refactors.

## Phase 0 — kernel convergence — COMPLETE

Pass280 froze the global kernel hostile/refactor campaign for the declared scope.

## Phase 1 — universal Rust runtime + public Rust facade — ACTIVE

Pass281 establishes `cfmd-runtime`; Pass282 extends the same vertical slice through creation/schema ownership; Pass337 adds the public application crate `cfmd` above that runtime boundary:

- durable open;
- immutable snapshots;
- facade-owned IDs and values;
- facade query IR;
- prepare-once query execution;
- atomic relation Plan commits;
- facade error taxonomy;
- no public kernel-type leakage;
- facade-owned schema/type/relation builder;
- durable create from a fully empty typed database;
- schema introspection.

Next acceptance work:

1. typed relation/domain handles — **implemented P283**;
2. object-first Rust domain mapping with stable textual keys and operation-produced Plans — **implemented P284**;
3. explicit reference/cardinality contracts and safe deep-path query DX — **implemented P285–P287**;
4. Plan sealing and Candidate preview — **implemented P290–P291**;
5. historical contexts/history/inverse — **implemented P292–P295**;
6. exact watch protocol — **implemented foundation P296–P298**;
7. public application crate — **implemented foundation P337**.

Acceptance gate: a Rust application can create/open, query, transact, preview and watch a durable CFMD database with one direct dependency: `cfmd`. `cfmd-runtime` remains the stable binding/runtime layer rather than the user-facing crate.

## Phase 2 — idiomatic Rust application surface

**P284 direction:** object-first is the primary application DX; relation-first remains the universal/dynamic escape hatch. Read queries stay `Query` values. Write operations over an object collection/query return inspectable `Plan` values; Plans compose and later feed Candidate/commit rather than acting as the primary mutable command bag.

Build typed/generated builders over the stable runtime IR rather than exposing raw numeric column coordinates as the final DX.

Targets:

- generated/static schema handles;
- typed `EntitySet<T>` / relation handles;
- deep relationship paths;
- typed query/result shaping;
- ergonomic Plan/Candidate APIs;
- executor-neutral async watch receive directly via `watch.next().await`, with executor-specific integration added only when it provides a measured capability beyond the standard Future/Waker contract;
- explainability/capability inspection.

## Phase 3 — Python facade

The retained [`CFMD_PYTHON_FACADE_THEORY.md`](CFMD_PYTHON_FACADE_THEORY.md) remains the detailed Python UX source.

Python bindings must translate to the Rust facade/runtime protocol. They must not:

- import internal kernel concepts;
- evaluate query semantics independently;
- implement Candidate/watch/history in Python;
- perform per-row FFI chatter when a batched runtime operation exists.

Python product targets remain familiar lazy query vocabulary, deep domain traversal, exact watch, Candidate future worlds, history/undo, and strong IDE typing.

## Phase 4 — local tooling / Studio

Expose the same Rust runtime protocol over a local authenticated transport for Studio/CLI/tools. External tools submit Plans and observe revisions; they never mutate database bytes behind the authoritative runtime.

## Phase 5 — additional language surfaces and release hardening

- .NET/WPF;
- native Rust ergonomic adapters;
- compatibility/semver CI;
- backup/restore UX;
- observability/resource limits;
- binary-size/performance regression budgets;
- platform durability certification expansion.

## Non-negotiable laws

- one authoritative Rust runtime owns semantics and writes;
- no hidden degradation from exact watch to polling;
- current/historical/Candidate worlds are explicit;
- many-valued traversal is explicit;
- no internal kernel ownership/layout leaks through public contracts;
- bindings share one runtime protocol instead of creating parallel database models.

### P286 — explicit cardinality foundation

Rust object-first entities support required refs, optional refs and explicit many-valued traversal. P342 supersedes the earlier reverse-backlink interpretation: `Many<T>` is now a first-class object relationship value and the object declaration is the public schema authority. The runtime lowers it into an internal typed edge relation; a target-side `Ref<Source>` is not required merely to make the relationship exist.

Materialized `Ref/Many` values are snapshot-bound and I/O-free until an explicit `load/all/where_/query/count/...` call. Detached `Many::new(...)` participates in graph insertion; bound Many values preserve relationships during scalar updates and detached replacements replace them atomically. Normal DX never exposes join/include/foreign-key concepts.

### P290–P291 — Candidate product surface

`Plan -> Candidate` is now the authoritative preview workflow. P290 added exact proposed-state object/relation queries, identity `get/require`, and commit of the same Plan intent. P291 adds typed candidate projections, exact effect summaries, certified-target/freshness diagnostics, and immutable preview summaries. Candidate stays queryable after the owning runtime closes; commit requires the bound runtime capability to remain open. Next product work can build history/undo/watch directly on this current/proposed revision vocabulary rather than introducing another state abstraction.

### P292 — durable causal history and exact Plan inverse

`Database::history()` is now backed directly by the kernel durable revision-effect ideal. Entries expose transaction identity, source/target revisions, causal prerequisites and exact relation changes. `HistoryEntry::undo_plan()` is available only for a live-head transition whose durable intent is exactly representable by the current Plan calculus; it returns an ordinary Plan, so proposed-state inspection still uses Candidate while publication remains an explicit database operation. A committed undo is itself history, making redo the same inverse operation over that compensating entry.

R&D payer discovered in P292: forward-exact mixed `DurableModelDelta` is not generally invertible because field patches retain target values without a complete source complement. Mixed object changes with no non-relation delta (for example scalar-only row rewrites) are exactly undoable now; lifecycle/reference-changing mixed effects are explicitly `ComplementRequired`. The next history pass should close that algebraic gap rather than add a fallback or a second product-side undo log.

### P293 — compact exact mixed-history complement

P293 closes that payer for newly committed mixed transitions. The durable transaction intent now atomically carries the exact source-relative forward model delta and target-relative reverse complement; preparation independently derives and validates both. `Plan` gained an internal explicit model-delta representation so a history inverse can stay an ordinary Plan even when lifecycle/carrier/reference state changes. Consequently `db.history()` / `Database::history()` can derive exact undo for entity creation/deletion and strong-reference changes after restart; preview remains inspectable before the database publishes the inverse. Undo-of-undo remains redo. WAL codec v11 is backward compatible with v10; legacy v10 mixed entries remain `ComplementRequired` rather than receiving a guessed inverse. Next payer: explicit non-head history rebase/conflict calculus, then exact watch over the same revision/effect vocabulary.

## P297 hosting/notification boundary

`watch()` delivery is transport-neutral. `PublicationNotifier` is a wake-only provider contract: first-party embedded use can keep the std in-process implementation, while hosted applications, UI event loops or future IPC/network services can explicitly install another provider. Provider notifications never carry authoritative database state and never decide writer compatibility; Revision/history and the kernel semantic conflict/rebase calculus retain those authorities. P347 also makes that provider the executor-waker authority: direct host `notify_waiters()` signals and runtime publication now reach blocking and async waiters through the same provider contract rather than through a bridge-local Waker registry.

Hosted CFMD should therefore be assembled as an explicit service composition (protocol + authentication + authorization + chosen transport/provider) rather than making `Database::open` start a listener. First-party transports may be shipped as optional modules, and application-defined providers remain an extension point.

## P298 watch lifecycle / backpressure foundation

The exact watch protocol now has deterministic cancellation and shutdown behavior without an async-runtime dependency. `WatchCancellation` is a wake-only lifecycle capability; it cannot mutate database state. `WatchStatus` exposes exact causal lag, while catch-up consumes durable history one revision transition at a time. There is intentionally no unbounded per-subscription event queue and no silent event dropping/recompute fallback. P347 hostile-tests cancellation/task migration/spurious-wake behavior plus 2,000 pending subscriptions. P348 removes the wrapper object entirely, keeps bounded catch-up on the same watch, and adds dependency-frontier wake filtering plus exact suppression of output-equivalent public events.

With watch semantics/lifecycle now mature enough for adapters, the next product boundary is hosted sessions: authenticated principals, capabilities/authorization and protocol ingress above `cfmd-runtime`. Transport implementations (local IPC, TCP/TLS, UI/event-loop adapters) remain optional providers rather than kernel responsibilities.


## P300 transport-neutral hosted protocol foundation

`cfmd-protocol` now sits above `cfmd-runtime` and consumes only restricted `SessionDatabase`. It owns language-neutral protocol DTOs and unary query/history/commit semantics, including base-revision concurrency binding, ingress resource limits and sanitized public errors. No transport/listener/authentication implementation is part of this layer.

## P301 exact hosted watch subscription protocol

Protocol version 2 now exposes exact server-side watch subscriptions: open returns a session-scoped `SubscriptionId` and exact initial result, blocking next returns one revision-tagged exact query-result delta, status exposes P298 lifecycle, and cancel/close/session-close terminate outstanding waits without polling. The protocol stores the existing runtime `QueryWatch` rather than materializing a second event queue or log, and maintained subscription count is bounded per session.

Next protocol work should define an explicit wire codec/framing + version/capability negotiation around the now-complete unary/watch semantic vocabulary. Concrete IPC/TCP/TLS remain optional transport adapters.


## P302 canonical hosted wire framing

The hosted protocol now has a canonical binary framing contract without selecting a transport. A fixed header permits magic/version/kind/request-id/payload-length validation before payload allocation. Wire v1 negotiates hosted protocol v2 and explicit capability bits. Payload DTOs are deterministic, bounded and fail closed on unknown/non-canonical encodings.

This keeps transport implementations intentionally thin: local IPC, TCP/TLS, QUIC, Studio and CLI move frames but do not own query/history/watch semantics, authorization or writer resolution. Next transport work can therefore focus on endpoint security/lifecycle rather than inventing another database protocol.

### Hosted composition foundation — P303

The hosted product path now has a transport-neutral `cfmd-host` layer. Future local IPC and network server providers should own only connection I/O/authentication evidence delivery and call this host boundary; they should not receive raw `Database` or implement independent permission/commit/watch semantics.


## P313 unified database lifecycle DX

The primary Rust construction model is `Database::builder(path)`. A new path resolves to single-file storage by default; directory storage is an explicit option or an existing directory detected on reopen. Database storage/runtime concerns (storage backend, future encryption, publication notifier) belong to this builder. Hosting is intentionally composed after open: importing `cfmd-host::DatabaseHostingExt` enables `db.host(authenticator, authorizer)` while preserving the crate dependency boundary.
