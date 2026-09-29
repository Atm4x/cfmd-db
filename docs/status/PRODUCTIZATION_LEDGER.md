# CFMD Productization Ledger

This ledger tracks the active product-facing work after the Pass280 kernel freeze. Kernel areas reopen only for concrete product/correctness evidence.

## CLOSED

- [x] Universal `cfmd-runtime` anti-corruption layer over `kernel-*`.
- [x] Create/open/schema/query/Plan/Candidate/history/historical worlds/exact-watch runtime semantics.
- [x] Hosted protocol/session/transport-neutral composition foundation.
- [x] Unified storage builder and single-file durability product path.
- [x] Secure-memory R&D04 mainline integration on Linux.
- [x] P337 public `cfmd` Rust application crate.
- [x] Object-first normal namespace plus explicit `cfmd::dynamic` escape hatch.
- [x] Public diagnostic category layer and no-kernel-dependency CI gate.
- [x] Public-crate end-to-end test/example using only `cfmd`.

## OPEN — immediate

- [x] P339 derive reverse-many declarations + introspectable RequiredOne/OptionalOne/Many cardinality metadata without storing collections in materialized objects.
- [x] P340 product-layer query-node/source identity, parent trace and diagnostic provenance across dynamic + object-first lowering.

- [x] P338 derive foundation: ordinary entity structs can generate schema/codec/symbolic accessors through `#[derive(CfmdEntity)]`.
- [x] P338 compile-fail gate for stable derive diagnostics (missing key, invalid identity type, missing identity); generated query diagnostics remain open.
- [x] P340 source-level public API compatibility contract and explicit pre-1.0 breaking-change policy (`docs/api/PUBLIC_RUST_API_COMPATIBILITY.md`).

- [x] P341 generated construction established inline `Many<T>` syntax.
- [x] P342 object-first relationship authority: `Many<T>` is a first-class relation value, requires no target-side backlink, graph insertion lowers to internal edge facts, `Ref/Many` materialization is explicit and snapshot-bound, update distinguishes preserve vs replace, and endpoint deletion is lifecycle-safe.
- [x] P343 relation-local `OwnedMany<T>` exclusivity, atomic ownership move, and explicit orphan policy.
- [x] P344 mutable relationship selections: filtered/ID bulk move-detach without target-object materialization plus derived Candidate preview effects.
- [x] P345 runtime-neutral async watch adapter boundary: stable subscription identity, one shared readiness-source identity per live runtime, executor-neutral generation wait, and bounded durable `drain_ready` without a second event queue.
- [x] P346-P348 executor-neutral watch Futures: race-free `Waker` registration with no executor dependency, polling fallback, helper thread or second event queue; P348 moves `next().await` directly onto runtime watches and deletes the transitional wrapper crate; Tokio compatibility is dev-only.
- [x] P347 async hostile/stress coverage: cancellation races, repeated/spurious wakes, task migration/Waker replacement, bounded lag fairness and 2,000 simultaneously pending subscriptions.
- [x] P348 dependency-frontier readiness + observable quotient: retain unavoidable O(K) delivery for K relevant subscribers while avoiding O(N) executor wakes for unrelated subscriptions; suppress output-equivalent empty public events without losing causal catch-up.

## P349/P350 Python hostile validation

- CPython 3.13 + PyO3/maturin drives the same exact Rust watch Future; Python is a consumer/test harness, not event authority.
- P349 adds replayable FFI delivery reservation so a cancel-vs-ready race cannot advance the Rust cursor and then discard the event before Python accepts it.
- P350 moves the harness onto `cfmd` only, exposing a missing `cfmd::dynamic::{QueryWatch, WatchEvent}` facade pair and locking that surface into the public compile contract.
- P350 also closes event-loop-shutdown retention by making in-flight Python receives Task-owned/cancellable; an abandoned bare Future must not retain a Rust receive after its loop closes.
- Black-box coverage now includes typed object CRUD/query/update/delete, deep reference traversal, exclusive ownership move/orphan preview, history undo, persistence reopen, typed object watches, queued receive cancellation, GC churn, 512 relevant pending watches and 256 unrelated pending watches.
- OPEN: multi-handle/process lifecycle, transaction/conflict/error mapping, larger relation/object stress and final Python DX/wheel design.

## OPEN — subsequent surfaces

- [ ] Final Python product binding/wheels over the public `cfmd`/runtime contract. P349/P350 validation harness is already proving asyncio and broad database semantics, but the final Python facade is intentionally not frozen yet.
- [ ] .NET/WPF adapter over the same runtime/watch protocol.
- [ ] CFMD Studio/CLI product polish over hosted/local transport.
- [ ] Backup/restore and corruption-recovery UX.
- [ ] Public benchmark/binary-size regression budgets.
- [ ] Platform support expansion (Windows secure memory intentionally deferred to native Windows work).

## Policy

- `cfmd` is the Rust application crate.
- `cfmd-runtime` is the universal runtime/binding semantic owner.
- `kernel-*` remains internal.
- Async runtimes are adapters, not semantic authorities.
- Relation-first/dynamic APIs remain explicit escape hatches rather than the default DX.


## Pass340 — query provenance + compatibility boundary

P340 gives every product query operation a process-local `QueryNodeId`, `QueryNodeKind`, source file/line/column and parent-node trace. Identity survives cloning and kernel lowering. `#[track_caller]` is propagated through typed/object-first wrappers so diagnostics name application callsites, not runtime implementation lines. Query preparation/evaluation errors retain provenance through `ErrorDiagnosticExt`.

The public SDK now has an explicit pre-1.0 compatibility policy and a compile-time consumer contract. P342 makes the object declaration the public schema authority: `Ref<T>` and `Many<T>` are real relation values, while relation/Γ edge structures are internal lowering. Relationship field access is I/O-free; explicit materialization/query methods are snapshot-bound. Detached object graphs lower into one Plan, bound Many values preserve relationships during ordinary scalar rewrites, detached Many values replace them, and kernel live-endpoint normalization removes dangling internal edges. The next SDK step is async adaptation over this object-first surface.

## P343 object ownership relation algebra

- `Many<T>` remains shareable: the same target may be linked from multiple owners.
- `OwnedMany<T>` is an exclusive object relation: within that declared relation, a target has at most one owner.
- `OwnedMany<T>` supports explicit edge mutation without target-object rewrites: `attach`, `detach`, `move_to`, and `move_all_to`.
- Moving ownership is one Plan over edge rows; it does not transiently orphan the target in the candidate world.
- `#[cfmd(orphan = "delete")]` opts an `OwnedMany<T>` field into `DeleteIfUnowned`; the default is `Keep`.
- Orphan evaluation happens after lifecycle normalization, so deleting an owner can delete newly orphaned targets while an atomic move preserves them.
- Ownership is currently relation-local. A future R&D pass may introduce explicit durable ownership domains if one target type must be exclusive across several different owner fields/types; do not infer that global rule implicitly.
- P346 closes the generic Rust async layer. P347 then found and removed a notifier-authority split: executor Wakers now register in the public `PublicationNotifier` itself, so direct host liveness/spurious signals wake blocking and async waiters identically. Executor-specific acceleration remains deferred; P348 is scoped to dependency-frontier readiness rather than a runtime-specific queue.


## P344 relationship-set mutation DX

- `Many::where_(...)` and `OwnedMany::where_(...)` return first-class relationship selections rather than dropping the caller into a read-only generic query surface.
- Selection `move_to` / `detach_all` projects only identities; target Rust objects are not materialized.
- `move_ids_to` / `detach_ids` validate source membership and fail closed instead of degrading move into attach.
- `delete_all` is intentionally stronger: it deletes target objects and therefore reads exact target rows, but still does not construct target Rust objects.
- `CandidatePreview::derived()` exposes normalization/orphan effects separately from explicit Plan row counts so Studio/CLI can explain policy-driven consequences before commit.

## P347 hostile async closure

- `PublicationNotifier` now owns both blocking wait and executor-Waker registration. The former P346 bridge-local Waker sidecar was incorrect for direct host `notify_waiters()` calls because those signals could bypass the sidecar; the split authority has been removed.
- One watch object exposes `next().await`, blocking `recv()`, `try_recv()`, and bounded `drain_ready(max_events)`; no conversion/wrapper boilerplate exists.
- Hostile coverage proves Waker replacement after executor task migration, pending receive cancellation, 128 repeated spurious wakes without fabricated database events, explicit 2+2+1 backlog slicing, and 2,000 simultaneously pending watches on one readiness source without lost publication.
- A publication relevant to all 2,000 subscriptions necessarily performs 2,000 task wakes; that is the O(K) delivery lower bound for K interested consumers, not by itself a defect. The avoidable case is unrelated subscriptions sharing the runtime source. P348 should therefore index readiness by exact maintained-query dependency frontier, not insert a generic dispatcher/event queue.

## P346 executor-neutral Future/Waker adapter

- `WatchReadiness::poll_after(observed, Context)` registers a standard-library `Waker` against the publication generation with an atomic generation recheck, so commit-between-read-and-register cannot be lost.
- Kernel publication wait handles own one waker-registration identity and unregister on drop; `WatchNext` clears a pending registration on future cancellation/drop, preventing abandoned task retention on a quiet database.
- Raw, object and projection watches all use the same direct `watch.next().await` path. Tokio is dev-only compatibility coverage; the product runtime has no executor dependency.
- The adapter does not buffer result events. A wake only causes the existing exact watch to re-read durable causal authority and advance its maintained state.

## P345 runtime-neutral async readiness/drain

- Every query/object/projection watch owns a stable process-local `WatchSubscriptionId`.
- `WatchReadinessSourceId` identifies the live runtime wake domain; watches from the same database runtime share that source identity even though cancellation remains subscription-local.
- `WatchReadiness::wait_after(generation)` remains the blocking wake primitive; P346 adds `poll_after(generation, Context)` for race-free standard-library Waker registration without a dispatcher thread.
- `drain_ready(max_events)` is bounded and nonblocking. It advances the existing maintained query state through durable causal history and returns the exact post-drain `WatchStatus`.
- No event queue, recompute path, polling path or Tokio dependency was added to `cfmd-runtime`. Durable history remains the only catch-up/backpressure authority.

## Pass351 — snapshot-bound Rust Transaction DX

- Product hostile inventory found that multi-operation writes still forced application code to manually carry `Plan + TransactionId + Candidate + Database::commit`, leaving exact-base composition discipline at the call site.
- `Transaction` is now a first-class `cfmd`/`cfmd-runtime` product type pinned to one exact live `ReadContext`. `Database::transaction(id)` and restricted `SessionDatabase::transaction(id)` create it.
- `tx.objects::<T>()` reads from the transaction base; ordinary object mutations continue to produce ordinary `Plan` values and `tx.apply(plan)` composes them only when database identity, exact source snapshot and authority all match.
- `tx.preview()`, `tx.candidate()` and `tx.commit()` reuse the existing `Plan -> Candidate -> commit` pipeline. No second mutation engine, auto-retry, implicit refresh, merge fallback or hidden rebase was introduced.
- Stale HEAD publication still fails `StaleRevision`; cross-snapshot composition fails `InvalidPlan`. This is an explicit optimistic transaction surface rather than a lock/SQL transaction emulation.
- `Plan::extend` exposes in-place exact-snapshot composition so transaction accumulation does not clone the aggregate Plan merely for DX.
- NEXT: continue Rust product hostile inventory. Highest open application-level gap remains ordered/range predicate algebra; it should be derived from pinned Γ order semantics rather than a generic SQL comparison fallback.
- Hostile follow-up: `Transaction::commit` borrows rather than consumes the transaction, preserving the exact plan/id pair for durable idempotent retry. Re-publishing the unchanged transaction returns the ordinary `AlreadyCommitted` outcome; uncertainty at the caller boundary therefore does not force plan reconstruction.

### P351 R&D — ordered predicate path (next payer)

Hostile inventory confirms that ordered predicates do **not** need a generic callback/post-filter fallback. `kernel-query` already treats Γ orderings as prepared semantic contracts for `TopKWithTies`: ordering IDs are collected, compiled against the pinned semantic context, and checked for congruence with the relation column equivalence. The missing piece is a first-class relational comparison node, not a new query subsystem.

The native path for the next pass is therefore: introduce one `FilterOrderConst`/comparison relation primitive parameterized by `{column, ordering, comparison, value}`; compile the pinned ordering during query preparation; validate ordering/equality congruence; apply the same predicate independently to inserted/removed deltas in maintained execution; then expose `Field::greater_than/greater_than_or_equal/less_than/less_than_or_equal` (and range sugar) only for object value types with a registered canonical ordering. Object schema generation can derive a stable per-field ordering identity next to the existing per-field equivalence identity and install the matching builtin Γ ordering (`BoolAscending`, `I64Ascending`, `F64Total`, `TextBinary`, historical entity-id ordering where semantically admitted). Optional/structural order is not to be guessed; it remains unavailable until a structural ordering contract is surfaced cleanly.

This gives O(delta) maintained filtering with no query recomputation and no SQL predicate AST/callback fallback. A future semantic range index may accelerate the same logical primitive without changing its public or mathematical semantics.
- Public-facade hostile follow-up also closed a session-construction leak: `Session::new` was public but its required `PrincipalId` remained only in `cfmd-runtime`. `cfmd` now re-exports `PrincipalId`, and the compile-time public API contract locks that vocabulary at the application facade.


### P353 — Ordered predicate DX + structural cleanup — CLOSED

The pre-release public abbreviations `gt/ge/lt/le` were removed in favor of `greater_than`, `greater_than_or_equal`, `less_than`, and `less_than_or_equal`. Internal Γ comparison vocabulary remains compact. P352-induced dispatcher/argument growth was mechanically split across query maintenance, transport, durable codec, physical plan execution, and object schema registration; strict Clippy is restored without `allow` suppressions or semantic changes.

### P352 — Γ-native ordered/range predicates — CLOSED

`FilterOrderConst { column, ordering, comparison, value }` is now first-class in the relational calculus, execution graph, differential program, maintained plan, linear-island normal form, transport, durable query codec and physical Plan. Preparation collects/compiles the pinned Γ ordering; typecheck requires matching ordering domain and ordering/equality congruence. Exact maintained execution filters incoming weighted deltas directly.

The object schema derives stable `cfmd.object.field-ordering.v1` identities and installs canonical primitive orderings for ordered scalar/identity fields. Product `Field` exposes `greater_than/greater_than_or_equal/less_than/less_than_or_equal` and inclusive `between`. Optional/structural values are deliberately not marked ordered. Public regressions cover all four comparisons/range and exact ordered watch behavior, including quotienting a non-matching revision without emitting an empty event. No SQL AST, callback predicate, Rust `Ord` authority, or recompute fallback was introduced.
