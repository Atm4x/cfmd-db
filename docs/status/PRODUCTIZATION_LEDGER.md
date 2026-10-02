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


## Pass354 — database-owned control surface

- Pass351's self-publishing `tx.preview()/tx.commit()` surface is superseded. `Transaction` is now a passive exact-snapshot change set: it may describe and accumulate intent, but it does not own publication authority.
- Normal terminal operations are `db.preview(&tx)` and `db.commit(&tx)` (or the restricted equivalents on `SessionDatabase`). The call site therefore names both the database being affected and the transaction being inspected/published.
- `tx.apply(plan)` was replaced by the less action-like `tx.add(plan)`: adding a plan changes only the transaction container, not the database.
- `Candidate::commit` was removed from the public model. Candidate remains an inspectable proposed future, not a second route around the database control surface.
- The old low-level `Database::commit(plan, id)` shape was renamed `Database::commit_plan(plan, id)` so advanced tooling remains possible without competing semantically with ordinary transaction publication.
- Session preview/publication now verifies that the transaction/plan was created under the same session authority. This closes a control-surface authority seam discovered by the DX audit.
- The next DX pass should remove manual Plan plumbing from ordinary entity writes. Target direction: collection-shaped mutation (`db`/transaction + entity set + value) while keeping Plan as the internal/advanced representation.

## Pass359–360 — ordinary mutation language convergence

- Entity CRUD, `Many` / `OwnedMany`, filtered relationship selections, and history undo now share one application mutation form: resource/database + `&mut Transaction` + payload.
- Ordinary relation/history mutation does not surface `Plan`; explicit `*_plan` and `HistoryEntry::undo_plan()` remain advanced representations for tooling/bindings/protocol tests.
- History undo is still the exact durable inverse and still uses kernel-certified non-head transport; the DX layer only changes where that existing Plan is accumulated.
- Public Γ-ordering hostile coverage now reaches `F64Total` edge values (`NaN`, signed zero, infinities) through both exact object queries and maintained watches, including suppression of non-matching revisions.
- NEXT: continue the outside-in DX audit for query composition/naming and expose additional already-proven kernel-query capabilities only where they can remain typed, exact, and incremental.

## Pass361 — Γ-native query composition + ordered boundary DX

- Outside-in hostile inventory found an already-proven kernel capability hidden behind `cfmd::dynamic`: exact/incremental `TopKWithTies` had no object-first Rust surface.
- `ObjectSet` / `ObjectQuery` and Candidate counterparts now expose `top(k, field)` and `bottom(k, field)` only for `OrderedObjectValue` fields with a declared canonical Γ ordering.
- The typed lowering validates relation ownership and ordering availability, then reuses the existing `Query::top_k_with_ties` / kernel-query maintained ordered-boundary calculus. No Rust materialize/sort stage, SQL `ORDER BY` emulation, recompute fallback, or second boundary engine was introduced.
- `ObjectPredicate::and` is native conjunction by sequential exact filter composition. It works across ordinary, ordered and deep relationship predicates without introducing callback evaluation.
- Public regression proves conjunction, tie-preserving top/bottom selection, and exact maintained watch deltas when the top boundary changes.
- Boundary semantics are explicit: tied rows at the k-th semantic ordering class are preserved, so result cardinality may exceed `k`; the operation selects a Γ boundary and does not claim physical result ordering.
- NEXT: continue outside-in query audit around projection/aggregation. Highest-value candidates are typed `distinct` and exact count/group aggregation, but only if their result typing and maintained semantics can be surfaced without materialization or a second generic aggregate path.

## Pass362 — set-native projection + exact count consolidation

- Hostile projection audit found that ordinary object relations are already kernel `Set`s: `Project` over a set canonically collapses Γ-equivalent projected rows. A proposed object-first `distinct()` was therefore rejected and removed as redundant SQL-shaped ceremony rather than added to the product surface.
- `ObjectSet::select(...)` and Candidate parity now expose typed set projection directly from the collection root.
- Object sets, object queries, projected queries, Candidate counterparts, `Many`, and filtered `ManySelection` counts now use one exact kernel `Group { group_columns: [], Count }` path. Previous SDK-side `rows().len()` / `ids().len()` count paths were removed.
- Public `top` / `bottom` are now the primary names for the stronger tie-preserving Γ boundary semantics; the application surface no longer leaks the internal `with_ties` qualifier. `TopKWithTies` remains the kernel/dynamic primitive.
- Hostile regression proves duplicate projected values are suppressed by maintained set projection itself and produce no empty public watch event; a novel projected equivalence class produces the exact delta.
- NEXT: continue from the same kernel `Group` machinery into typed grouped aggregates (`group_by(...).count()` and exact numeric aggregates) only if one typed result-shape law can cover exact execution and maintained watch without a parallel aggregate engine.

## Pass363 — multiplicity-preserving projection + typed kernel Group

- Hostile DX review overturned the Pass362 product interpretation of `Project(Set)`: although set projection is mathematically valid, ordinary language-level `select` is expected to preserve one output occurrence per selected source object unless the developer explicitly requests quotienting.
- No host-language bag or provenance fallback was added. Typed object `select` lowers as kernel `PromoteToBag -> Project`; therefore duplicate projected values remain distinct occurrences while all equality semantics still come from Γ.
- Explicit `ObjectProjectionQuery::distinct()` and Candidate parity lower to the existing maintained kernel `Distinct` using the projected columns' declared equivalences. Ordinary watch emits multiplicity deltas; distinct watch suppresses changes that do not change a semantic class.
- `group_by(key).count()` now lowers directly to kernel `Group + ExactCount`; `group_by(key).sum(f64_field)` lowers to kernel `Group + ExactF64Sum`. Live and Candidate result decoding share the same typed group-key law and no second aggregate engine exists.
- PASS362's exact zero-key count and `top` / `bottom` consolidation remain valid; only the product interpretation of projection multiplicity is superseded.
- NEXT: expose maintained grouped aggregate watches through one typed aggregate query/result abstraction, then evaluate a first-class kernel cardinality operator for `one` / `one_or_none` instead of host-side full materialization or a hidden `LIMIT 2` workaround.
## Pass367 — field-to-field Γ equality and wide typed query shapes

- Closed the `FilterEqColumns` facade gap without a second predicate engine: `field.eq(value)` remains unchanged while `field.eq(other_field)` lowers to the kernel node. Kernel equivalence refinement validates the shared semantic contract even when per-field semantic IDs differ.
- Added exact maintained negation as `ObjectPredicate::not()` / `Field::ne(...)` via kernel `Difference`; `or` was deliberately not synthesized from nested differences because a mathematically valid but structurally poor plan would violate the no-sugar-at-any-cost rule.
- Removed the accidental three-column ceiling from typed projection/group keys. Heterogeneous Rust tuples are supported through arity 12; homogeneous fixed-width keys/projections use const-generic field arrays. Executor/runtime group width remains vector-native and unchanged.
- Exact/Candidate/watch field-equality regression and public 4-column/array shape regression pass.
- NEXT: continue the query-kernel inventory, with priority on relationship-count comparison algebra and other maintained operators that currently require weaker application-side terminal forms.


## Pass368 — zero-total relationship cardinality algebra

- `ManyField::count()` now exposes one typed comparison law (`eq`, `ne`, ordered comparisons and inclusive `between`) rather than equality-only special cases.
- Zero-degree owners are represented algebraically rather than by synthetic rows: positive degrees come from maintained `Group(ExactCount)`; predicates accepting zero use anti-join against violating positive groups, predicates rejecting zero join only matching positive groups.
- Exact, Candidate and maintained execution share the same Group/Join/AntiJoin/Difference machinery; no host map, outer-join emulation, callback validator or SQL fallback exists.

## Pass369 — native Γ-aware Union / disjunction

- `RelExpr::Union` is first-class across exact evaluation, differential/maintained execution, execgraph, transport, durable query codec and physical planning.
- Set union tracks Γ support from both branches and emits output only on support zero-crossings; Bag union adds multiplicities. Public `ObjectPredicate::or` lowers directly to this operator rather than a De Morgan/Difference synthesis.
- Exact/Candidate/watch hostile coverage proves overlap does not duplicate Set rows and irrelevant revisions do not fabricate events.

## OPEN — current DX / semantic-rule line after Pass369

- [x] **Deep typed traversal/navigation (P370, strong-reference predicate path).** Nested `ref.matches(|x| ...)` ceremony is no longer required for strong-reference filtering: generated symbolic paths lower to the same Join/Project/Distinct algebra and perform no hidden object I/O. Nullable optional-reference traversal remains intentionally explicit until a null-path law is chosen; deep projection is a later expression-shape extension rather than a hidden load.
- [x] **Predicate ergonomics (P370).** Typed predicates expose idiomatic Rust `&`, `|`, `!`; conjunction remains exact sequential filtering, `|` remains P369 native Union and `!` remains native Difference. No callback evaluator or alternative boolean engine exists.
- [ ] **Semantic Rules / field validation calculus.** Audit `kernel-semantics`, violation/schema/change owners and historical rule machinery, then expose one typed database-owned rule language for user rules such as regex/range/membership constraints applied to fields. Invalid values/candidates must fail commit through database semantics, not through an application callback. Reuse the same semantic-expression substrate for entity invariants / transaction preconditions only where the underlying kernels genuinely support the common law.
- [x] **Certified change/commit closure (P371–P373, ordinary transaction classes).** `db.commit(&tx)` now preserves kernel rebase certificates through relation-only and mixed/object residual publication. Stable client intent is distinct from realized effect; retry identity, history, undo and recovery consume the correct authority. Snapshot-bound/full-revision/schema-migration operations remain intentionally exact rather than being silently transported.
- [ ] **Schema/codegen frontend.** Deliberately deferred until the runtime/query/rule surface is stable; future schema syntax should generate this model rather than dictate a parallel one.


## Pass370 — symbolic deep traversal + predicate operator DX

- `CfmdEntity` now generates one symbolic path wrapper per entity. Strong-reference accessors accumulate a typed path, so ordinary filters can read `person.passport().region().code().eq("RU")` instead of nesting `matches` closures.
- Path construction performs no object read. The leaf predicate is lowered once, then the path is folded back through the existing exact Join/Project/Distinct calculus; root row shape and Γ equivalences are preserved.
- Value leaves retain ordinary equality and canonical ordered/range predicates. Direct reference `matches` remains available as compatibility/advanced syntax. Optional-reference traversal is deliberately not guessed: `None` semantics remain explicit until a nullable-path law is designed.
- Typed predicate values now implement Rust `&`, `|`, `!`. These are syntax over the existing predicate algebra only: AND remains filter composition, OR is the P369 native Γ-aware Union, NOT is Difference.
- NEXT: hostile-audit `kernel-change -> Transaction -> db.commit(&tx)` while simultaneously inventorying `kernel-semantics` / `kernel-violation` / schema hooks for the OPEN Semantic Rules line. Do not create a validator callback framework or a second merge engine.

## Pass371 — hostile certified-commit audit + Γ Set residual primitive

- Cross-kernel audit traced the ordinary stale commit path through `kernel-change`, `kernel-plan::certify_transition_rebase`, `cfmd-runtime::exact_rebased_plan`, durable transaction intent, and retry recovery.
- Confirmed existing strength: runtime history already preserves `RewriteActionLaw` for relation classes and model/lifecycle coordinates; `StrongCommute` and `SameIdempotentIntent` are both classified coordination-free by `infer_write_action_law` rather than falling back to revision-number staleness.
- Found the next real loss of strength: after certification, product rebase still replays the original raw `RelationDelta` on the newer HEAD. For Set effects this is weaker than the certified action law: an already-satisfied `EnsurePresent` / `EnsureAbsent` must residualize to zero rather than be replayed as a duplicate raw mutation.
- Added `RelationDelta::residualize_against` in `kernel-query` as the Γ-native residual operator for this boundary. Set actions are compared by pinned canonical classes, including removal of the current semantic representative rather than Rust-value equality; Bag multiplicity remains exact and is not silently weakened to Set semantics.
- Hostile prototype proved that simply wiring this residual into `db.commit(&tx)` is still insufficient: durable retry identity currently records the **realized physical delta**. Once residualization changes that delta, retrying the same original transaction id conflicts with the residual realization. The prototype integration was intentionally reverted rather than weakening retry/recovery semantics.
- Therefore certified commit needs one explicit durable distinction before residual publication: **client semantic intent** versus **realized residual effect**. The former is the stable idempotency identity; the latter is what is physically applied to the certified newer HEAD and what causal/history replay must retain.
- OPEN next step: carry this dual-intent authority through `DurableTransactionIntent` / WAL / recovery / runtime commit, then wire `RelationDelta::residualize_against` into `exact_rebased_plan`. Only after that should `SameIdempotentIntent` become a real automatic merge in `db.commit(&tx)` rather than merely a readiness certificate.
- Semantic Rules remains OPEN and unchanged: audit/rule work must not be mixed into the durable-intent change until commit identity is structurally correct.

## Pass372 — dual durable relation intent + certified residual commit

- Closed the P371 retry-identity blocker for relation-data certified rebases by making the distinction explicit in durability: `RelationDataResidualExact` retains canonical **client mutations** separately from the exact **realized residual mutations** applied to the newer HEAD.
- WAL/metadata/recovery codecs persist both authorities. `same_client_intent` compares original client mutations, while runtime history and inverse/replay surfaces consume realized mutations. The same transaction id therefore remains idempotent even when certification shrinks the physical effect.
- `DurableRuntime::commit_derived_relation_data_residual` publishes a residual while binding retry identity to the original relation request. Ordinary relation-data retry also recognizes a committed residual realization.
- `Database::commit(&tx)` now uses P371 Γ `RelationDelta::residualize_against` for certified pure relation-data rebases. Already-satisfied Set `EnsurePresent`/`EnsureAbsent` actions disappear; an empty residual is still durably represented by a new no-op revision so transaction identity survives later state changes and restart.
- Hostile public regression covers two concurrent identical `Many::attach` intents: the second is `Rebasable`, commits an exact residual, retry returns `AlreadyCommitted`, only one edge exists, and reopen preserves the result.
- Mixed/model/object residual publication remains OPEN: do not generalize until client-vs-realization authority is extended to `MixedRevisionExact` with exact model residual/complement history.
- Semantic Rules / field validation calculus remains OPEN and carried forward unchanged.

## Pass373 — mixed/object residual intent + exact recovery complement

- Extended the P372 client-vs-realization split to mixed/object commits with `MixedRevisionResidualExact`: canonical client relation/model effects remain stable retry identity while realized relation/model effects describe the publication against the certified newer HEAD.
- Certified mixed publication starts from the current authoritative revision, Γ-residualizes Set relation actions, applies the already-certified model delta, then recomputes the exact realized `DurableModelDelta` and inverse complement relative to that current source. No second object merge engine exists.
- Runtime history consumes only realized relation/model effects and the realized complement. Hostile coverage proves two concurrent first object inserts sharing carrier creation merge, retry idempotently, undo only the second realized object effect, preserve the first object/carrier, and reopen exactly.
- The ordinary adaptive transaction commit boundary is therefore closed for relation-only and mixed/object mutation classes under the existing `kernel-change` certificate. Snapshot-bound transactions, schema migrations and full-revision replacement remain exact by design rather than participating in adaptive transport.
- **NEXT / OPEN:** Semantic Rules / field validation calculus remains the next product R&D line. Audit existing semantics/violation/schema owners before adding regex/range/membership/custom rules; rules must be database-owned typed semantics, not Rust callbacks.


## Pass374 — database-owned Semantic Rules foundation

- Hostile audit confirmed the real invariant authority: every ordinary future revision is rebuilt through `kernel-validation`; `kernel-violation::ViolationMeasure` already provides the exact non-negative witness algebra, but user-defined field rules did not previously exist in schema authority.
- Added typed `FieldRule` primitives for integer inclusive ranges, Unicode-scalar text length bounds, and finite text membership. Multiple rules on one target are conjunctive schema contracts.
- Rules can target both kernel `FieldDef` values and relation columns. P374 temporarily exposed object-rule attachment through a builder call; P375 supersedes that DX by carrying the same authority directly in `CfmdEntity` field metadata.
- `kernel-validation` enforces the same rules during revision/Candidate construction and also emits exact `DynamicViolationWitness` entries for VMF derivation. A violating value is therefore rejected by database semantics independently of Rust application callbacks.
- Checkpoint codec v3 persists both target classes. Public hostile regression proves a valid value commits, an invalid Candidate is rejected as `InvariantViolation`, and the same rule remains authoritative after reopen.
- **OPEN carry-forward:** regex/pattern rules, richer compositional/custom rule expressions, and generated/schema-file ergonomics. Do not add a host-language callback validator. A regex operator should arrive only with a deterministic DB-owned pattern engine/module contract that can be serialized, recovered, and shared by all bindings.
- **NEXT:** continue the rule calculus from these schema/validation owners; inspect whether regex should be a pinned semantic module or a dedicated deterministic kernel rule VM. Preserve the existing `FieldRule` target model and VMF zero-law.

## Pass375 — entity-owned rule DX + explicit schema-root R&D

- [x] Removed the temporary `SchemaBuilder::object_field_rule::<E>(name, rule)` product API completely. It required a second, stringly attachment step after the entity had already declared the field and made rule ownership visually ambiguous.
- [x] `CfmdEntity` field metadata is now the ordinary Rust authority for database-owned scalar rules:
  - `#[cfmd(range(min = ..., max = ...))]` on `i64`;
  - `#[cfmd(length(min = ..., max = ...))]` on `String`;
  - `#[cfmd(one_of("...", "..."))]` on `String`.
- [x] The derive emits rules into `ObjectFieldSchema`; object registration maps them to the real object-relation columns. Kernel validation, Candidate/commit rejection, VMF witnesses and durability therefore remain the sole enforcement path from P374.
- [x] Rule arguments are literal schema data. Arbitrary Rust expressions are rejected so schema authority cannot depend on host callbacks/config evaluation. Wrong rule/type pairings fail at derive time.

### Explicit schema-composition DX R&D

Autodiscovery/global registration is rejected. A process may host multiple databases with intentionally different entity sets (for example production telemetry vs test), so the developer must explicitly choose the membership of every database schema.

Variants considered:

1. **Chained `SchemaBuilder::object::<T>()` calls.** Mechanically explicit, but duplicates the later `db.objects::<T>()` vocabulary, scales poorly to large schemas, gives collection names no first-class type identity, and encourages schema assembly to look like runtime mutation/configuration. Keep only as low-level/dynamic assembly while the typed root is developed.
2. **Type tuple / `entities::<(User, Passport, ...)>()`.** Explicit and compact, but loses stable collection names and cannot naturally become `db.users`; therefore it does not solve the object-first facade problem.
3. **Function-like `schema! { users: User, ... }` DSL.** Can generate a good facade but introduces a second Rust mini-language before the type model is stable, weakens ordinary IDE/refactor transparency, and overlaps the later optional `schema.tmd` frontend.
4. **Global derive/linker inventory.** Rejected: membership becomes implicit and process-global, making multiple databases with different schemas surprising and unsafe.
5. **Named typed schema root with `EntitySet<T>` fields.** Preferred direction. Example target shape (not implemented in P375):

```rust
#[derive(CfmdSchema)]
struct ProdSchema {
    users: EntitySet<User>,
    passports: EntitySet<Passport>,
    telemetry: EntitySet<Telemetry>,
}
```

Why this currently wins:
- every included entity is explicitly visible in one database-specific type;
- field names are simultaneously schema membership names and the future typed collection facade, so no `Entity<User>` marker has to be mentally converted into a list later;
- order is declarative, not dependency-sensitive: the assembler first collects the complete selected entity set, then resolves `Ref<T>` / relationship requirements and fails closed for missing targets;
- the same entity type can participate in several schema roots, while test/prod roots can intentionally differ;
- large schemas can later compose named sub-roots/modules without any global registry;
- it leaves room for a typed database wrapper/facade where `db.users` is an actual bound `EntitySet<User>`, not an abstract metadata token.

Important unresolved implementation detail before introducing `CfmdSchema`: decide the clean ownership/binding shape between the untyped durability/runtime `Database` and the typed root so `EntitySet<T>` is a real collection facade without self-references, per-access allocation, or hidden snapshot semantics. The likely direction is a typed database wrapper that owns the raw database plus a bound root and exposes the root by ordinary Rust deref/field access, but this must be benchmarked/reviewed before mainline adoption.

### Carry-forward

- **OPEN:** deterministic `Matches`/regex rule semantics and richer compositional rule calculus. No host regex callback.
- **OPEN / NEXT schema DX:** prototype and hostile-review the explicit `CfmdSchema { field: EntitySet<T> }` ownership/binding model before replacing ordinary `SchemaBuilder::object::<T>()` use.
- **OPEN:** entity/model invariants and transaction `require` should reuse the same DB-owned semantic expression/rule authority rather than become parallel validator systems.


## Pass376 — explicit typed schema root + live `EntitySet<T>` database facade

- Hostile-reviewed the P375 schema-root candidates against the actual `Database`, `ObjectSet`, snapshot, reopen and multi-database ownership model. The named typed root remains the preferred design; tuple/macro/global-registration variants stay rejected for the reasons recorded in P375.
- Added `#[derive(CfmdSchema)]` for explicit database membership. Every schema field must be a named `EntitySet<T>`; the same entity type cannot appear twice. No entity is auto-discovered merely because `CfmdEntity` exists in the process.
- `EntitySet<T>` is a live collection handle, not an unbound metadata marker and not a retained `ObjectSet<T>` snapshot. Bound sets share one `Arc<Database>` authority; field access allocates nothing and each query/read takes the current committed snapshot only when the operation begins.
- `SchemaDatabase<S>` owns the raw database authority plus one bound schema surface and exposes that surface by ordinary Rust field access, so application code uses `db.users`, `db.tasks`, etc. Core `preview`, `commit`, history and revision operations remain database-owned. `raw()` is an explicit tooling/hosting escape hatch rather than the ordinary object path.
- `S::database(path)` returns a structured typed builder. Storage/encryption/notifier options remain available, while schema membership itself cannot be mutated by builder calls. Create derives the kernel schema from `S`; open verifies full kernel definitional equivalence, including field rules and semantic modules, before binding the typed facade.
- Strong/optional `Ref<T>` fields now contribute explicit target-relation requirements during object schema registration, matching the already strict `Many<T>` dependency law. Missing referenced entities fail schema construction instead of remaining latent until data insertion. Registration order is still irrelevant: requirements are resolved after the complete explicit member set is assembled.
- Hostile public coverage proves reversed dependency order, missing-reference failure, exact typed reopen, rejection of a different schema root over an existing database, and two intentionally different schema roots hosted in one process.
- Ordinary example code now uses `TodoSchema { todos: EntitySet<Todo> }` and `db.todos`; chained `SchemaBuilder::object::<T>()` remains only the dynamic/advanced assembly layer for tooling/tests/bindings.

### Carry-forward

- [ ] **Schema composition for very large models.** Design named sub-roots/modules (for example flattened schema fragments) only if they preserve explicit membership and direct `db.<set>` ergonomics; do not reintroduce global discovery.
- [ ] **Deterministic `Matches`/regex Semantic Rule.** Remains OPEN from P374/P375; choose one serialized DB-owned pattern semantics, not a host regex callback.
- [ ] **Compositional entity/model invariants and transaction `require`.** Reuse the same semantic-rule authority only after the common expression law is demonstrated.
- [ ] **Optional-reference traversal and deep projection.** Keep explicit until nullable-path and result-shape semantics are proven rather than hidden behind ORM-style I/O.

## Pass377 — authoritative Entity capability + partial-Context hostile design

### Agreed DX direction

- Physical `Database` and typed application context are separate concepts. A persisted CFMD database owns its complete authoritative schema and can be opened/hosted in a process that has no generated Rust entity model at all. A future typed `Context<M>` binds an application model to that already-open database; hosting must not require recompiling every entity type into the host binary.
- `CfmdSchema` remains an explicit developer-selected composition root. Global/linker/autodiscovery remains rejected because one process may deliberately host multiple databases with different models.
- `CfmdEntity` has one stable semantic identity via `#[cfmd(key = "...")]`. Different Rust structs in different consumers may use the same key while knowing different subsets of the persisted entity.
- No `partial` marker is introduced. Whether a Rust descriptor is "partial" is contextual and becomes misleading when the same complete struct is merely used under a restricted role. Ordinary `CfmdEntity` means "the typed contract this code knows".
- `#[cfmd(authoritative)]` is an explicit opt-in on the **entity itself**, not on the schema root. It means this local struct is allowed to participate as a complete persisted definition when a new database is created. This gives a developer reading the entity source an immediate local signal about which definition is intended as database authority.
- P377 adds `AuthoritativeObject` as a derive-emitted capability. `CfmdSchema` derives a balanced type-level authority tree from its members, and `DatabaseDefinition` is implemented only when every member is authoritative. Consequently `.create()` exists only for schema roots whose entities all opted into `#[cfmd(authoritative)]`; ordinary schema roots remain valid typed contracts/open surfaces. The balanced tree avoids a linear trait-recursion depth for large schemas.
- The P376 `ProdSchema::database(...)` / `SchemaDatabase<S>` entry shape is now considered transitional proof of the `EntitySet<T>` ownership model, not final application DX. Target separation remains `Database` (physical/runtime authority) plus a typed Context surface.

### Partial entity contract law

Target context compatibility is **not** full definitional equality:

`local CfmdEntity contract ⊆ persisted authoritative entity`

A consumer may intentionally omit persisted fields. Example: the authoritative `User` may contain `passport`, while a reader process may define the same semantic entity key with only `id`, `name`, `doctor`, and `colleague_comment`. Adding or changing unrelated persisted fields should therefore be close to free for old consumers.

Omitted fields are not sent merely because the session could theoretically read them. The target query/materialization path should project only the fields actually named by the bound local contract/query. This is both a data-minimization/security improvement and the cleanest schema-evolution law.

### Hostile findings — why naive subset binding is unsafe

P377 deliberately does **not** turn current `ObjectSet<E>` into subset binding. Current object relations are position-addressed at the facade boundary: generated accessors use local field positions, `RowCodec` expects the local row width, and full-object mutation encodes/replaces complete rows. Simply accepting a shorter local struct would therefore be wrong:

1. Omitting a middle persisted field can shift every later local field onto the wrong persisted column.
2. `E -> E` whole-row update from a shortened struct can erase or replace hidden persisted values.
3. `add(E)` from a shortened struct is under-specified when omitted persisted fields are required, have rules, or participate in relationships/invariants.
4. `remove(E)` is a full-row operation today and is not a correct primitive for a partial materialization.
5. A hidden `Ref<Passport>` must not force the reader binary to define/materialize `Passport` merely because the authoritative entity has that reference.

A regression therefore keeps same-key shortened entities fail-closed under the current exact `ObjectSet`/P376 typed-open path. Partial Context support must arrive through a distinct semantic projection binding rather than by weakening an exact-shape check.

### Required safe binding/mutation model

- Bind local fields to persisted columns by stable semantic field identity, never by coincidental ordinal position. The existing per-field semantic identities/equivalences derived from `(entity key, durable semantic field name)` provide useful substrate, but the facade needs an explicit local-to-persisted column map and projected row decoder before subset materialization is enabled.
- Reads/query predicates compile against that map and request only referenced/projected columns.
- Partial writes must be **field-coordinate patches** over the stored authoritative row. Unmentioned columns are preserved exactly. They must not lower through local full-row replacement.
- Creating a new persisted entity from a partial contract must stay unavailable unless a separate complete/default-proven creation law exists. `#[cfmd(authoritative)]` is the current safe capability for typed database definition; future server-side defaults may justify a distinct create contract, but missing values must never be silently invented by the SDK.
- Delete-by-identity/query can be made safe for a partial context because the database can remove the authoritative stored row internally, but delete authority is independent from entity shape and must be checked explicitly.
- Candidate validation always runs against the **complete persisted schema**. A reader/writer does not need to know a hidden field rule or cross-entity invariant in order to be subject to it. If a visible-field patch would make the full candidate invalid, preview/commit fails through normal DB semantic authority.
- Certified rebase/merge must remain coordinate-based. Omitted fields must not become synthetic writes/conflicts; only actually mutated semantic coordinates participate, while full candidate validation may still reject a combination that violates an invariant.

### Security hostile result

A shortened Rust `CfmdEntity` is a DX/compatibility boundary, **not an authorization boundary**. P377 audit found current session permissions are intentionally coarse (`Read`, `Write`, etc.); after obtaining a `ReadContext`, the dynamic query surface is not field/relation-role aware. Therefore "passport is absent from the reader struct" cannot by itself protect passport data from a malicious/raw client.

Before partial Context is considered security-capable, authorization must be DB-owned and enforced on the query/change IR itself, including dynamic/binding clients. Target authority dimensions include at least:

- relation/entity read;
- individual field read (and therefore traversal/projection eligibility);
- individual field mutation;
- entity create;
- entity delete;
- historical/watch equivalents where applicable.

Typed Context shape and runtime role authority remain orthogonal:

- Context shape: what this binary can express/materialize conveniently and compatibly.
- DB role/session authority: what this principal is actually allowed to read/change.
- Persisted authoritative schema/rules: what exists and which states are valid.

A reader may use a full local entity descriptor and still be runtime read-only; conversely, an omitted field is not a security grant or denial by itself. The server/DB must reject unauthorized IR before execution and must never rely on client-side omission or deserialization dropping a secret field.

### Semantic Rules carry-forward

- Existing P374/P375 field rules remain DB-owned and authoritative regardless of Context omissions.
- Target custom Semantic Rules should be user-authored conditions over the typed query/predicate algebra but compiled into deterministic serialized CFMD semantic expressions, not arbitrary Rust callbacks. Field sugar (`range`, `length`, `one_of`, future deterministic `matches`) should converge toward that same rule calculus where the kernel law supports it.

### P377 implementation/status

- [x] `#[cfmd(authoritative)]` entity marker implemented.
- [x] Typed schema `.create()` compile-gated through inferred `DatabaseDefinition`; non-authoritative schema roots compile as contracts but do not have a callable create method.
- [x] Compile-hostile check confirms a reader schema fails at `.create()` with the missing `AuthoritativeObject` / `DatabaseDefinition` bound.
- [x] Same-key shortened entity regression remains fail-closed instead of accidentally receiving positional subset semantics.
- [ ] Replace P376 exact typed-open concept with `Database` + typed `Context<M>` binding.
- [ ] Implement semantic local-field -> persisted-column projection map and partial row materialization.
- [ ] Design field-coordinate patch mutation for partial contexts; do not reuse full-row `update(E -> E)`.
- [ ] Add DB-owned relation/field/operation authorization on query/change IR before treating Context omissions as part of least-privilege deployment.
- [ ] Deterministic regex/custom Semantic Rule calculus remains OPEN after the Context/security ownership line is structurally safe.

## Pass378 — partial Context semantic projection + safe scalar patch foundation

### Closed from P377

- [x] **Physical Database + typed Context binding exists.** `Database::context::<S>()` binds a typed consumer surface to an already-open database without asking that consumer schema to recreate or equal the persisted database definition. Raw `Database::open()` / hosting therefore remains schema-model independent.
- [x] **Semantic local-field -> persisted-column projection map.** Partial Context fields are matched by durable object field equivalence derived from `(entity key, durable semantic field name)`, never by ordinal coincidence. Omitted middle columns therefore cannot shift later local accessors onto the wrong persisted fields.
- [x] **Partial row materialization.** Context object queries start from the persisted relation and project only local semantic fields before decoding the Rust entity. The authoritative hidden columns are not decoded into the consumer value. Raw `Database::objects::<E>()` deliberately remains exact-shape for tooling/low-level callers.
- [x] **Full-row mutation is fail-closed for shortened contracts.** `add`, value-based `remove`, query `update(E -> E)`, and full-row plan construction reject projected/non-exact object sets before mutation intent is formed. A shortened Rust value is never interpreted as the complete persisted row.
- [x] **First semantic field-coordinate patch primitive.** `EntitySet::set(tx, id, |e| e.field(), value)` updates one ordinary scalar field by semantic field identity. The database reads the complete authoritative stored row internally, substitutes only that persisted column, and carries every omitted column unchanged. Consumer code never needs to model the hidden values.
- [x] **Persisted rules remain authoritative over partial writes.** Hostile regression intentionally omits an authoritative `doctor_note` length rule from the reader descriptor. The reader can form a scalar patch, but commit of an invalid value is rejected by the complete persisted schema; the secret omitted field and prior valid value remain unchanged.

### Hostile safety conclusions

- Context projection is a compatibility/data-minimization boundary, not an authorization boundary. P377's requirement for DB-owned field/relation/operation authorization remains mandatory before roles can rely on omitted fields for secrecy.
- Projection is safe because every derived `CfmdEntity` contains an identity field; set projection therefore cannot collapse two different entity identities merely because hidden fields were omitted.
- Partial create remains unavailable: omitted required/reference/rule-constrained values are never invented.
- Partial full-row replace remains unavailable: omitted columns never become synthetic writes.
- Scalar patch currently excludes identity/reference/lifecycle fields. Reference mutation has model/liveness side effects and must not be faked as a relation-cell replacement; it needs an explicit semantic reference-patch law.
- Current scalar patch preserves hidden columns exactly but internally forms a full-row relation delta. This is correct and fail-closed, but a later refinement should make certified rebase/conflict coordinates field-granular so two independent field patches do not conflict merely because the storage relation row is shared.
- A `Ref<T>` field that is actually present in a local contract can still expose traversal through the underlying read context. Context membership is therefore not yet an authorization boundary; future DB-owned IR authorization must police traversal as well as direct field projection.

### DX status after P378

The emerging separation is now executable rather than only conceptual:

```text
Database
    persisted authoritative model / durability / hosting
        |
        +-- context::<ConsumerSchema>()
                |
                +-- EntitySet<LocalEntityContract>
                        reads = semantic projection
                        scalar writes = semantic field patch
```

`#[cfmd(authoritative)]` remains the entity-local opt-in that allows a type to participate in persisted database definition. An ordinary same-key `CfmdEntity` remains a consumer contract and may contain all or only some persisted fields; there is still no misleading `partial` marker.

### OPEN after P378

- [ ] **Finalize Database/Context public naming and creation DX.** P376 `ProdSchema::database(...)` / `SchemaDatabase<S>` remains transitional creation scaffolding. Do not freeze final names until raw Database + Context + builder/host composition is reviewed together.
- [ ] **Reference/relationship patch calculus.** Extend semantic patches to `Ref<T>`, optional refs and relationship moves only through the existing lifecycle/object model authority, never raw cell substitution.
- [ ] **Field-granular certified change coordinates.** Preserve P372/P373 dual-intent semantics while ensuring independent partial field patches can rebase at semantic field coordinates rather than whole persisted rows.
- [ ] **DB-owned granular authorization.** Enforce relation/entity read, field read, traversal, scalar/reference field write, create, delete and history/watch authority on query/change IR, including raw/dynamic clients.
- [ ] **Deterministic `Matches`/regex + general Semantic Rule calculus.** Resume after Context/auth ownership is structurally safe.
- [ ] **Large schema composition ergonomics.** Keep explicit developer membership; introduce sub-roots only if they preserve direct `ctx.<set>` ergonomics and no global discovery.
## Pass379 — stable semantic field names + rename-safe Context binding

### Closed

- [x] Added `#[cfmd(rename_from = "old_name")]` on stored `CfmdEntity` fields. The current Rust identifier remains the source-level accessor/materialized member name; the previous name becomes the durable semantic field name. Ordinary fields without the attribute keep `Rust name == durable semantic name`.
- [x] `ObjectFieldSchema` now carries both names explicitly. Every semantic coordinate derived by the product layer was audited and moved to the durable name: field equivalence, structural inner equivalence, canonical ordering, kernel `FieldId` for references, symbolic relation semantics, exact object binding, partial Context projection, and semantic scalar patch resolution.
- [x] Name lookup for generated Rust accessors still uses the current Rust identifier. Therefore renaming changes source ergonomics without changing the database coordinate.
- [x] Derive rejects empty/redundant `rename_from`, use on virtual `Many`, and two stored fields resolving to one durable semantic name. Silent alias collisions are fail-closed at compile time.
- [x] Hostile regression creates an authoritative entity whose `medical_note` field declares `rename_from = "doctor_note"`, then binds an old same-key Context that still calls the field `doctor_note`. The old reader materializes the value, patches it, and the authoritative reread observes the change through `medical_note`. A persisted length rule attached to the renamed authoritative field still rejects an invalid patch formed through the old name. The same compatibility is tested across close/reopen, proving no in-memory alias table is involved. A second hostile regression renames a strong-reference field and proves the legacy reference accessor/predicate still resolves the same target coordinate after reopen.

### Architectural result

P379 confirms that partial Context is improving the semantic substrate, not merely adding a facade feature. Field order was removed as identity in P378; P379 also removes the current source spelling as identity. Human-facing names resolve into durable semantic coordinates before query/change work reaches the kernel. This makes additive schema evolution and explicit field renames compatible with old Context binaries without adding a second query engine or validation path.

The current `rename_from` contract intentionally preserves one durable origin name. If a field is renamed again, the attribute must continue to name that durable origin until/if CFMD later persists an explicit alias-history table. P379 does **not** guess rename chains or fuzzy-match names; a typo therefore fails binding rather than aliasing to the wrong field.

### OPEN after P379

- [ ] Finalize physical `Database` vs typed `Context` creation/open/host naming; P376 `SchemaDatabase` creation scaffolding remains transitional.
- [ ] Reference/optional-reference/relationship semantic patches through lifecycle/object authority.
- [ ] Field-granular certified change coordinates so independent partial patches merge/rebase by actual semantic field rather than shared physical row.
- [ ] DB-owned granular authorization over query/change IR: field read, traversal, field write, create/delete, history/watch. Context omission remains a compatibility/data-minimization boundary, not a security role.
- [ ] Deterministic regex/`matches` and general serialized Semantic Rule calculus.
- [ ] If schema evolution eventually needs multiple historical names per field, design persisted alias history explicitly; do not infer aliases from string similarity.


## Pass380 — kernel-first schema migration calculus + strict authoritative/client split

### Architectural decision

- `#[cfmd(authoritative)]` now means a strict current persisted definition. Authoritative entities may not carry compatibility metadata.
- P379 `rename_from` is superseded and removed from the intended model. Schema evolution is not encoded as legacy tags on the current entity.
- A non-authoritative consumer may use `#[cfmd(bind = "persisted_name")]` to keep any local field spelling while binding to the current authoritative coordinate. This is local Context metadata only and never pollutes persisted schema authority.
- Persisted schema changes are first-class migrations: Old schema -> explicit deterministic migration model -> New schema. Frontends/CLI may provide different syntax, but runtime/kernel receive one serialized/typed migration model rather than host callbacks.

### Kernel work closed

- [x] Added `Schema::migration_base_equivalent`: a migration may replace field/relation/rule/presentation coordinates while requiring the non-data-bearing type/capability/structural foundation to remain compatible.
- [x] Added `SchemaMigrationTransport`, a verified one-step migration capable of changing fields and relations atomically while rebuilding a trusted target Revision and running full target validation.
- [x] Field migration is target-defined and supports passthrough, drop-by-omission, constant creation, split (one source -> many targets), merge (many sources -> one target), and deterministic transforms through `ExactQuery`.
- [x] Added migration-only relation row rewrites. They transform selected source columns into every target column with typed `ExactQuery`, avoiding pollution of the maintained/live relational query algebra with a migration-only row-map operator.
- [x] Added deterministic kernel-query `I64ToF64`, allowing the concrete hostile case `i64 -> f64` to compile and execute inside the kernel with no host callback.
- [x] Target Revision is always rebuilt through `kernel_revision::Revision::build` / `kernel-validation`; migration code cannot bypass current authoritative rules.

### Runtime bridge closed

- [x] Added low-level `cfmd-runtime::MigrationModel` plus field/relation/column rules and `MigrationValueExpr`. This is intentionally runtime vocabulary, not final Rust/Python frontend sugar.
- [x] `Database::migrate(model, transaction, history)` verifies the model against the live source schema, builds the target Revision through `SchemaMigrationTransport`, and publishes via the existing durable `SchemaMigrationExact` path.
- [x] Close/reopen regression proves the migrated schema revision is durable.
- [x] Historical inversion is deliberately not faked. P380 exposes only explicit `MigrationHistoryPolicy::Forget`; reversible complement compilation remains OPEN.

### Hostile conclusions / OPEN

- [ ] General reversible migration complements. Compile exact migration-specific complements/retention instead of silently backing up the whole DB or pretending arbitrary transforms are reversible.
- [ ] Entity/type/lifecycle structural migrations. P380 intentionally keeps type/capability/lifecycle foundations fixed; changing entity identity/carrier structure needs explicit identity/lifecycle transport laws.
- [ ] Richer deterministic scalar migration expression vocabulary as demanded by real migrations (casts/parsers/options/products), always DB-owned and serializable.
- [ ] Frontend DSL/codegen/CLI. Rust SDK, Python and TMD should compile into the runtime migration model; none should become a second migration engine.
- [ ] Continue Context work after this foundation: reference/relationship patch calculus, field-granular change coordinates, then DB-owned granular authorization.
- [ ] Deterministic regex/general Semantic Rules remain OPEN.

## Pass381 — first-class semantic migration boundary; undo != historical materializability

### Hostile finding

P380 already persisted schema migrations in the canonical causal effect ledger, but the runtime history layer classified `SchemaMigrationExact` only as `NonPlanTransition`. `revision_at()` consequently omitted migration edges from its exact-plan reconstruction graph. That behavior exposed a conceptual conflation: inability to express an inverse as an ordinary current-schema `Plan` is not the same property as inability to materialize a retained historical world. This is exactly the migration/history distinction recorded in the post-P380 continuation design.

### Closed

- [x] Added durable `SemanticChangeEvent` as a projection of the existing committed `DurableRevisionEffectRecord`. No second migration/history log was introduced.
- [x] A semantic change event records causal effect identity, transaction epoch/id, source/target revision, source/target schema revision, migration lens/spec identity, semantic pins and encoding version.
- [x] Added `HistoricalBoundaryAuthority` independent from Plan reversibility and forward-transform invertibility: `LocalComplement`, `ExternalArchive`, `ExplicitlyForgotten`, `LocalPayloadReleased`.
- [x] `RuntimeHistoryEffect` now carries the semantic boundary projection for schema migrations while retaining `NonPlanTransition` for ordinary undo semantics.
- [x] Public `cfmd-runtime::HistoryEntry` now exposes `HistorySemanticChange`; application history can therefore identify a migration as one semantic revision boundary rather than infer it from physical rewrite work.
- [x] Regression proves the P380 `380 -> 381` migration appears as one `SchemaMigration` history entry with a `380 -> 381` semantic change event and explicit forgotten historical authority.
- [x] Kernel regression proves local/external retention authority is orthogonal to transform invertibility and distinguishes already-released/forgotten history.

### R&D conclusion

The correct next primitive is **not** a generic inverse of the current B world. CFMD needs a retained **historical epoch anchor** on the source side of a semantic migration boundary. The anchor may be backed by an old checkpoint/generation, a compact archive representation, or a proven complement/materialization object, but it must name the source historical world and participate in retention/GC authority. This allows `db.at(before_migration)` to enter schema epoch A directly rather than evaluate `B -> inverse migration -> A`.

The event/complement metadata alone is intentionally not claimed to be a complete historical snapshot. `HistoricalBoundaryAuthority::LocalComplement` means the migration complement authority is locally retained; whole-world materializability still requires the epoch anchor/coverage law below.

### OPEN / next

- [ ] Define `HistoricalEpochAnchor` / equivalent source-world authority keyed by revision/schema epoch without duplicating a second history engine.
- [ ] Bind anchor lifetime to durable generation/checkpoint retention so physical compaction cannot delete the last representation required by a retained historical revision.
- [ ] Make `revision_at()` select the appropriate historical epoch anchor across `SemanticChangeEvent` instead of attempting inverse migration of current HEAD.
- [ ] Prove crash cases: before semantic publication, after semantic publication before physical rewrite, during mixed A/B physical materialization, and after compaction.
- [ ] Replace P380's prototype-only `MigrationHistoryPolicy::Forget` with an explicit retained-history policy once the epoch-anchor authority exists; do not fake this by serializing an arbitrary full DB backup into a generic migration callback/complement.
- [ ] After epoch retention is closed, develop semantic cutover + background physical rewrite and certification barriers for migrations that require preparation before cutover.

## Pass382–Pass387 — retained migration epochs and semantic/physical cutover split

### Closed through P386

- P382 introduced `HistoricalEpochAnchor`: a retained migration source world is pinned to the physical generation that can reconstruct its source revision/schema. Directory compaction cannot collect pinned generations; single-file initially failed closed where it could not retain them.
- P383 taught historical reconstruction to cross a schema boundary by entering the pinned source epoch and replaying that epoch, never by applying an inverse migration to the current world. Historical WAL inspection became read-only/certified rather than taking writer ownership.
- P384 removed an over-conservative single-file barrier: an anchor into the active generation already survives native compaction because the active generation image is relocated as authority.
- P385 added native immutable single-file historical closures carried across checkpoint generations as authenticated sections. Carry-forward is streaming and retains only generations referenced by live anchors.
- P386 added explicit irreversible release of `HistoricalEpochAnchor` authority, encrypted historical-closure coverage and torn-root/reopen fault tests. Semantic migration history and complements remain independent from physical old-world retention.

### Pass387 — semantic cutover frontier is causal, physical materialization is operational

Hostile audit found that P380 migration publication still conflated two different costs: `Database::migrate` eagerly constructs target Revision B and WAL persists exact target bytes, but checkpoint materialization can lag behind the committed semantic cutover. P387 makes that already-existing split explicit without inventing another progress journal.

- [x] Added `SchemaMigrationPhysicalState` / `MigrationPhysicalAuthority`.
- [x] `WalForwardCutover` means the migration effect is committed and B is authoritative while the active checkpoint causal ideal still does not contain that migration effect.
- [x] `NativeCheckpoint` means the migration effect is present in the active checkpoint's causal ideal. The criterion is causal membership, **not numeric comparison of RevisionId**.
- [x] Physical progress is derived from the canonical causal ledger + checkpoint frontier + source epoch anchor. No mutable migration-progress state is persisted.
- [x] Added `materialize_pending_schema_migrations`: all pending cutovers collapse into one checkpoint publication and the operation creates no new semantic revision/effect. Calling it after the frontier is already native is a no-op.
- [x] Kernel-plan exposes the same operational state/materialization primitive so scheduling can be added above durability without changing history semantics.
- [x] Regression proves WAL-cutover state survives reopen, native checkpoint materialization changes only physical authority, causal effect identity remains byte-for-byte the same, and reopen observes the native frontier.

### OPEN after P387

- [ ] **Remove the remaining O(data) schema-migration WAL payload.** `SchemaMigrationExact` still stores a complete encoded target Revision. This is now the dominant blocker to true lazy physical A -> logical B operation. Persist the deterministic verified forward migration program + target semantic context instead, so recovery can reconstruct B from physical A without a full target snapshot in the migration WAL.
- [ ] Once the durable forward program exists, make reads over still-A physical regions project through that certified transport and make new writes B-native; then background materialization can monotonically eliminate A regions/chunks rather than merely rotating a full B checkpoint.
- [ ] Add crash/recovery matrix for partially materialized A/B regions after the O(data) WAL snapshot is removed.
- [ ] Distinguish migrations whose target validity can be certified before cutover from migrations requiring a global preparation/certification phase.
- [ ] Then return to Context: reference/relationship patches and field-granular certified change coordinates, followed by granular authorization.

## Pass388 — durable deterministic schema-migration program; O(data) target snapshot removed

### Hostile finding

P387 made semantic cutover vs checkpoint materialization explicit, but `SchemaMigrationExact` still serialized the complete transformed target `Revision B` in PREPARE. That made semantic cutover O(data) even though the checkpoint could remain in epoch A.

### Closed

- [x] Added canonical `kernel-transport::SchemaMigrationProgram`: target `SemanticContext` plus deterministic field/relation rewrites. It contains no source or target database state.
- [x] Production durability now stores `SchemaMigrationProgram` in `SchemaMigrationExact` / `DurableRevisionChange::SchemaMigration`; the production descriptor constructor does not accept a target state.
- [x] Added canonical codec for migration programs, including target semantic context, scalar `ExactQuery` expressions and relation query/row rewrites.
- [x] WAL recovery in `kernel-plan` now verifies the decoded program against the causal source revision and executes `SchemaMigrationTransport` to reconstruct the target revision. No full target snapshot is required.
- [x] `kernel-plan::migrate_schema` independently certifies that the supplied program reconstructs the exact requested target before durable PREPARE, preventing target/program split-brain.
- [x] Explicit schema migration may transport to a new semantic environment; ordinary typed transports still reject silent semantic-environment reinterpretation. The target world is rebuilt and fully validated under the target context.
- [x] Hostile regression builds a 5,000-entity target and proves migration PREPARE remains compact relative to full target encoding, round-trips as a program, and contains `DurableRevisionChange::SchemaMigration` rather than `FullRevision`.

### Architectural result

The durable semantic boundary is now:

```text
causal source Revision A
+ serialized deterministic SchemaMigrationProgram
+ target semantic context / semantic module authority
+ migration complement / history authority
        |
        v
verify program against A
        |
        v
logical authoritative Revision B
```

Migration WAL size is now a function of migration-program/schema complexity, not database cardinality. This closes the dominant P387 O(data) cutover bottleneck without introducing a generic delta fallback or second migration engine.

### OPEN after P388

- [ ] Introduce native mixed-representation coordinates so physical A regions can remain A after semantic B cutover while reads project through the persisted forward program and new writes are B-native.
- [ ] Background materialization must monotonically replace A regions/chunks with B representation and derive progress from physical authority, not a second semantic history state machine.
- [ ] Crash/recovery matrix for partially materialized A/B regions, including compaction and reopen.
- [ ] Define preparation/certification barriers for migrations whose target invariants cannot be certified from local/lazy projection alone before cutover.
- [ ] Then resume Context reference/relationship patches and field-granular change coordinates, followed by granular authorization.

## Pass389 — migration forward-slice calculus + bounded-memory physical row cursor

### Hostile finding

P388 removed the O(data) target snapshot from migration WAL, but recovery still reconstructs a complete concrete `Revision B` because `Revision` owns a complete `DatabaseState`. Merely tagging chunks as schema A/B would therefore be decorative: the logical layer would already have paid the full A -> B transform before physical execution could exploit mixed representation.

### Closed

- [x] Added verified `MigrationRelationSlice` coordinates derived directly from `SchemaMigrationTransport`: `Passthrough`, `RowLocal`, and general deterministic `Query`.
- [x] General query slices expose their exact source-relation dependency set from the query IR. They are no longer an opaque generic migration path.
- [x] Added `materialize_relation_slice`, allowing one target relation to be derived independently from source-epoch state without constructing the rest of target B.
- [x] Added `required_source_relations(native_target_relations)`: the exact source-epoch retention frontier is derived from the forward program plus actual B-native physical coordinates, not persisted as a second migration-progress journal.
- [x] Regression proves the retention frontier is monotone and dependency-exact: once all remaining target slices stop depending on one A relation, that source relation may be released independently.
- [x] Added `transform_row_local_slice`, a one-source-row -> one-target-row primitive for bounded-memory background rewrite. General query slices reject this operation explicitly rather than silently routing through a generic row-map fallback.
- [x] `kernel-plan::RowLocalMigrationCursor` streams any native source layout one row at a time through a verified row-local migration slice. Regression uses native i64 columnar source and yields target f64 rows without source `DatabaseState` materialization or relation-sized transient input buffering.

### Architectural result

The mixed-representation unit is now a verified relation slice, not an arbitrary byte range and not a SQL-style fallback:

```text
SchemaMigrationProgram
        |
        v
verified SchemaMigrationTransport
        |
        +-- target relation R1 : Passthrough(A.R1)
        +-- target relation R2 : RowLocal(A.R2 -> B.R2)
        +-- target relation R3 : Query({A.R1,A.R4} -> B.R3)

actual B-native target coordinates
        |
        v
required_source_relations(...)
        |
        v
exact A-retention frontier
```

For row-local slices, physical rewrite can already be streamed with O(row) transient transformation memory. Query slices remain explicit relational programs with declared dependencies; they are not misclassified as row-local just to gain a convenient storage path.

### OPEN after P389

- [ ] `Revision` still requires a complete concrete target `DatabaseState`. Introduce a target-semantic logical view / relation authority boundary so runtime reads may resolve an unmaterialized B relation through its A slice without eagerly building all of B.
- [ ] Bind actual physical relation coordinates to source-schema/target-schema epoch authority in `kernel-plan`/durability and derive the `native_target_relations` set from published physical artifacts, never from a separate progress bitmap.
- [ ] Publish row-local B-native regions/chunks incrementally and reclaim A regions according to `required_source_relations`; then extend the same ownership law to general query slices.
- [ ] Add crash/recovery/compaction tests for a genuinely mixed generation containing both A-backed and B-native relation coordinates.
- [ ] Define preparation/certification barriers for migrations whose target invariants require global proof before semantic cutover.
- [ ] After mixed physical migration is closed, return to Context reference/relationship patches and field-granular certified change coordinates, then granular authorization.

## Pass390 — semantic revision head + mixed migration revision view

### Hostile finding

P389 provided exact migration relation slices and bounded-memory row-local physical transformation, but the authoritative `kernel-revision::Revision` still bundled semantic identity with a complete concrete `DatabaseState`. Treating A/B storage state as metadata on that object would therefore remain cosmetic: constructing authoritative B would still require eager materialization of all B relations.

### Closed

- [x] Added `kernel_revision::RevisionSemanticHead`: a certified `RevisionId + SemanticContext` authority that validates the pinned semantic environment but deliberately does **not** claim a complete materialized `DatabaseState`.
- [x] Full `Revision` remains the stronger fully materialized/validated certificate and can project its `semantic_head()`, preserving strict write/validation paths while separating read-side semantic authority.
- [x] Added `kernel_plan::MixedMigrationRevisionView`, whose sole semantic authority is target head B while relation realization is derived from actual native B coordinates plus the verified migration program.
- [x] Added `MixedMigrationRelationCoordinate::{NativeTarget, ForwardFromSource}`. Coordinate selection is deterministic from physical authority + verified forward program; it is not retry/fallback routing.
- [x] Native target relations are read directly under B. Passthrough A relations are exposed without rewrite. Row-local A relations stream through `RowLocalMigrationCursor`. General query slices materialize only their explicit source dependency set and execute their prepared deterministic relational program.
- [x] `MixedMigrationRevisionView::required_source_relations()` delegates to the P389 dependency frontier using the actual native-B relation set. When the final dependent target relation becomes native, source A authority becomes collectible without a progress bitmap.
- [x] Construction fails closed when a semantic head not equal to the migration target context attempts to authorize the view.

### Architectural result

```text
RevisionSemanticHead B
        |
        +-- semantic authority: exactly B
        |
        +-- relation R1: NativeTarget(B.R1)
        +-- relation R2: ForwardFromSource(RowLocal A.R2 -> B.R2)
        +-- relation R3: ForwardFromSource(Query {A.R4,A.R5} -> B.R3)

actual native B relation ids
        |
        v
required_source_relations(...)
        |
        v
exact retained A frontier
```

For the first time in the migration line, a certified authoritative semantic B object exists without requiring construction of `DatabaseState B`. This is intentionally a read-only logical/physical view: ordinary `Revision` write/validation machinery remains fully materialized until mixed-state commit certification is designed rather than weakened implicitly.

### OPEN after P390

- [ ] Bind the view's `source_relations` / `native_target_relations` to real durable `PhysicalStore` relation authorities and schema-epoch identities instead of caller-supplied maps.
- [ ] Introduce mixed-state write certification: new B writes must publish B-native coordinates immediately while untouched A-backed coordinates retain their verified forward slice.
- [ ] Define validation/certification law for target rules that span multiple mixed relations; do not weaken full target invariants merely because data is lazy.
- [ ] Publish row-local B-native regions incrementally and reclaim A according to the derived dependency frontier; general query slices keep explicit dependency scheduling.
- [ ] Crash/recovery/compaction matrix for a genuinely mixed generation whose durable physical authority contains both A-backed and B-native relation coordinates.
- [ ] Once mixed migration is closed, return to Context reference/relationship patches, field-granular certified change coordinates, and granular authorization.

## Pass391 — certified mixed physical authority + cutover invariant closure

### Hostile finding

P390 separated semantic head B from complete `DatabaseState B`, but its mixed view still accepted caller-owned maps of source/native-target relations. That left two correctness gaps: physical progress was not owned by the real `PhysicalStore`, and a caller could supply a B-native relation whose bytes did not equal the already validated target world. Re-running full global validation on every lazy read would fix correctness by introducing an O(data) read-path bottleneck.

### Closed

- [x] Added `MixedMigrationCutoverCertificate`. It can be created only from a fully validated source `Revision A`, fully validated target `Revision B`, and verified `SchemaMigrationTransport`, and requires exact `P(A) == B` before sealing.
- [x] The cutover certificate retains Γ-canonical `RelationBaseWitness` values for source and target relations. Target-wide/global invariants therefore remain discharged by ordinary `Revision` validation at cutover; lazy physical representation is allowed only when it realizes the same certified logical bases.
- [x] Added `MixedMigrationPhysicalAuthority` backed by the real `PhysicalStore + LayoutBinding` coordinates. `MixedMigrationRevisionView` no longer accepts caller-owned relation maps.
- [x] Physical authority certification checks required A-backed source layouts against source witnesses and every published B-native layout against target witnesses. A physically different B relation fails closed before the mixed view can be served.
- [x] The expensive relation scan/witness comparison occurs once when physical authority is certified, not on every view/read. The steady read path uses already sealed physical coordinates.
- [x] The P389 `required_source_relations()` frontier is now derived from actual target layouts present in certified `PhysicalStore` authority rather than a caller-maintained native-target set.

### Architectural result

```text
validated Revision A
        +
verified migration P
        +
validated Revision B
        |
        | require P(A) == B
        v
MixedMigrationCutoverCertificate
        |
        +-- certified A relation bases
        +-- certified B relation bases
        |
        v
PhysicalStore
        +-- A.R2 @ source layout
        +-- B.R1 @ native target layout
        +-- B.R3 @ native target layout
        |
        v
MixedMigrationPhysicalAuthority
        |
        v
MixedMigrationRevisionView(B)
```

The global-validation law is therefore not "validate lazy data later". B is validated once before semantic cutover, then every mixed physical coordinate is proven to represent the exact certified A/B relation base appropriate to the migration program. Physical materialization can change representation without changing the logical world.

### OPEN after P391

- [ ] Persist the mixed physical layout/epoch coordinates through the durable generation descriptor/recovery path so reopen derives `MixedMigrationPhysicalAuthority` from durable authority rather than runtime construction.
- [ ] Add mixed-state write certification: a write under semantic B must publish the touched relation as B-native while preserving certified A-backed authority for untouched relations.
- [ ] Row-local background materialization should publish B-native layouts incrementally and reclaim A layouts only when `required_source_relations()` releases them.
- [ ] Add crash/recovery/compaction matrix for durable generations that genuinely contain source-epoch and target-epoch layouts simultaneously.
- [ ] General-query slices keep explicit dependency scheduling; do not route them through the row-local cursor.
- [ ] Once durable mixed migration is closed, return to Context reference/relationship patches, field-granular certified change coordinates, then granular authorization.

## Pass392-P393 — migration architecture hostile / Physical Realization Algebra decision

### Supersession

The P389-P391 mixed-current-world direction is no longer the target architecture. Its useful forward-slice/dependency primitives are retained, but `MixedMigrationRevisionView` / `MixedMigrationPhysicalAuthority` are R&D prototypes, not the desired steady-state semantic/runtime boundary.

The selected invariant is:

```text
one current semantic world
+
PhysicalAtoms P
+
certified RealizationRoot ρ : P -> current finite model
```

For migration `M : A -> B`, current semantic authority changes atomically to B and the target realization is compositionally derived:

```text
ρ_B = normalize(M ∘ ρ_A)
```

Old physical atoms may remain as leaves of `ρ_B`; schema A does not remain a current read/runtime authority. Materialization is representation normalization under one semantic revision, and GC is reachability from current + retained historical roots.

### Consequences

- [x] No current-world query/change/watch/auth layer should branch on "physical schema A vs B".
- [x] Rename/default/drop/type-transform may change realization without rewriting all physical payload immediately.
- [x] Shadow epochs / exact effect transport remain optional preparation/certification machinery for global migrations, not the default storage model.
- [x] P388 compact durable `SchemaMigrationProgram` remains valid and useful.
- [x] P389 forward slices/dependency calculus remain useful as realization compiler/materializer primitives.
- [ ] Replace mixed migration runtime prototypes once the realization calculus can cover their useful behavior.

## Pass394 — stable semantic relation-column identity

- [x] Relation columns now have stable semantic IDs distinct from ordinal positions.
- [x] Migration column rewrites use semantic column IDs rather than `usize` identity.
- [x] Durable migration/checkpoint codecs preserve semantic column IDs.
- [x] Runtime positional convenience lowers to semantic IDs before the kernel boundary.

This is foundational for realization: semantic coordinates may survive reorder/layout changes while physical ordinals remain lowering details.

## Pass395 — in-memory Physical Realization reference calculus

- [x] Added independent `kernel-realization` owner rather than embedding realization semantics in `kernel-plan`.
- [x] Added `PhysicalAtomId`, explicit `PhysicalCodec`, typed atom payloads and an in-memory `PhysicalAtomStore`.
- [x] Added `RealizationRoot` covering lifecycle, carriers, fields and relations, with scalar field expressions `Direct`, `Constant` and deterministic `ExactQuery` transform.
- [x] Added exact evaluation back to `DatabaseState` as the current semantic oracle and `certify_realization` fail-closed equality checking.
- [x] Added representation-rewrite certification: a derived field can be materialized into a new physical atom while proving the semantic `DatabaseState` is unchanged.
- [x] Added `RealizationDependencyGraph` by semantic coordinate plus reachability union across current/historical roots; this is the reference GC law.
- [x] Durability is intentionally unchanged. Full `Revision=(S,Γ,M)` remains durable authority until realization composition/materialization laws are mature.

### Next

- [ ] PASS396: compile schema migration composition `ρ_B = normalize(M ∘ ρ_A)` for identity/rename/default/drop/type-transform and then split/merge relation cases.
- [ ] Reuse `kernel-transport` / `kernel-query` expressions; do not grow a parallel scripting language inside `kernel-realization`.
- [ ] Only after composition is stable: same-revision physical materialization publication, writable-lens / exact-overlay lowering, then durable atom/root authority.

## Pass396 — migration/realization composition + performance hostile

- [x] `SchemaMigrationProgram` now composes into a target `RealizationRoot` without first materializing a complete target `DatabaseState`.
- [x] Field split/merge/default/drop semantics compose through `Product` + existing `ExactQuery`; no second expression language was introduced.
- [x] Row-local relation migration composes into a relation realization over the retained source relation atom; general relational query rewrites remain fail-closed until a proper compositional relation realization exists.
- [x] The resulting target root evaluates exactly to the existing `SchemaMigrationTransport` target-state oracle in regression tests.
- [x] Added representation-only relation materialization alongside scalar-field materialization.
- [x] Release-mode microbenchmarks were added and run. Generic unmaterialized scalar transform is ~12-13x slower than a direct in-memory atom read (~203 ns vs ~15-17 ns on this sandbox); after materialization the direct path returns to ~16 ns/native baseline.
- [x] Scale hostile: composing the current per-cell reference root for 100k entity field values took ~139.6 ms and retained O(N) atom/dependency metadata. Therefore the P395/P396 per-cell root is explicitly a correctness oracle/reference calculus, not the production storage granularity.

### Architecture consequence

Production realization MUST be factorized at physical column/segment/chunk granularity. Semantic cutover may not allocate one realization expression per stored value. A hot derived coordinate MUST converge to native realization (on-access/hotness/compaction/background are scheduling choices); permanent generic transform-on-read is not an acceptable steady-state hot path.

### Next

- [ ] PASS397: factorized physical realization rules over column/segment atoms, preserving P396 composition laws while making cutover metadata O(schema/layout), not O(data).
- [ ] Reuse stable `RelationColumnId` / field semantic IDs from P394 and existing `LayoutBinding`/columnar physical owners.
- [ ] Benchmark direct native, derived cold-read, first-access materialization and steady-state native throughput before any durability integration.
- [ ] Keep full `DatabaseState`/current Revision as oracle until factorized realization is proven equivalent.

## Pass397 — factorized field-column realization + performance gate

- [x] Added factorized field-column payloads in `kernel-realization`: one physical column segment stores many `(EntityId, Value)` pairs instead of one `PhysicalAtom` per stored field value.
- [x] Added `FactorizedRealizationRoot` / `FactorizedFieldRule`; semantic field identity is the key, while one realization rule serves an entire physical column segment.
- [x] `SchemaMigrationProgram` field migrations now compile to factorized realization metadata without enumerating entities. Split/merge/default/drop semantics reuse existing deterministic `ExactQuery` algebra.
- [x] `normalize(M ∘ ρ)` recognizes direct alias/copy, typed constants, and direct `i64 -> f64` column conversion. These cases avoid per-read `Value::Product`/`BTreeMap` construction.
- [x] Factorized materialization replaces one derived field-column rule with one native column atom under the same semantic revision.
- [x] Relation rewrites intentionally fail closed in the factorized compiler until the relation-column/segment compiler is implemented; the new path never silently falls back to the P396 per-row realization.
- [x] Correctness regression compares factorized migration evaluation against the existing P396/per-cell migration-realization oracle.

### P397 performance hostile

100,000 entities, one migrated `i64 -> f64` field, release build, warm runs:

```text
compose metadata:      46.31–51.26 µs
source physical atoms: 3
target dependencies:   3

direct point read:     33.11–34.07 ns
normalized derived:    28.32–30.65 ns
ratio derived/direct:  0.85–0.93x

whole-column materialization (100k values): 14.07–16.52 ms
post-materialization point read:            31.84–37.12 ns
ratio native/direct:                        0.95–1.09x
```

The P396 O(N) cutover blocker is therefore removed for factorized field columns: migration composition metadata is independent of row cardinality for this class. The earlier ~12-13x generic transform-on-read penalty is also removed for the normalized direct `i64 -> f64` column rule.

### Architecture consequence

Factorization is now the mainline production direction. The P395/P396 per-cell root remains a correctness oracle only. Production physical realization should continue toward column/segment/chunk rules with normalization into compiled direct transforms. Generic `ExactQuery` realization remains valid as a cold/rare bridge but is not the preferred hot representation when a verified transform can be compiled into a specialized rule.

### Next

- [ ] PASS398: factorized relation-column/segment realization using the stable relation-column IDs from P394; support passthrough and row-local transforms without materializing `Vec<Row>`.
- [ ] Add bounded chunk subdivision policy so on-access materialization need not rewrite an entire huge field column; chunking must remain a physical scheduling decision, not semantic state.
- [ ] General relational query migration remains fail-closed until a factorized dependency/materialization plan can represent it without current-schema-A routing.
- [ ] Only after factorized fields + relations share one realization calculus should durability begin moving from full `DatabaseState` snapshots toward durable realization authority.

## Pass398 — factorized relation-column realization + performance gate

- [x] Added factorized relation-column physical atoms keyed by the stable semantic column identities introduced in P394; row ordinals remain reconstruction/lowering coordinates only.
- [x] Row-local `SchemaMigrationProgram` rewrites compile to one realization rule per target relation column rather than one expression per row or a materialized `Vec<Row>`.
- [x] Normalization recognizes direct column reuse and direct `i64 -> f64`; relation-column materialization replaces only the selected derived column with one native physical atom under the same semantic revision.
- [x] Reorder hostile exposed and fixed a P394 continuity bug: `TypedRelationTransport` / `SchemaMigrationTransport` passthrough classification now requires stable relation-column IDs/order to match in addition to `RelationDef` equality.
- [x] General relational query rewrites remain explicit fail-closed in the factorized compiler; there is no fallback to the P396 row-buffered/current-schema-A path.
- [x] 100k-row release benchmark confirms O(schema/layout) composition metadata for the tested row-local transform: ~63.98–73.30 µs composition, 2 source atoms / 2 target dependencies including lifecycle, derived scan in the same cost class as direct scan, and ~3.70–4.11 ms whole-column materialization in warm runs.

### Architecture consequence

Factorized fields and row-local relations now share the same production realization law: stable semantic coordinate -> factorized physical atom(s) + normalized deterministic rule. The P395/P396 per-value/per-row forms remain correctness/reference machinery only. Migration cutover for these classes no longer scales with stored row count.

### Next

- [ ] Add bounded physical chunk subdivision under a factorized column rule so first-access materialization of a very large column need not rewrite the whole column; chunking remains physical scheduling, not semantic state.
- [ ] Develop factorized general relational-query realization/preparation using explicit dependency closures; keep it fail-closed until an exact non-fallback plan exists.
- [ ] Add writable current-schema overlays/lenses over factorized realization so B writes never require inversion of non-injective migration transforms.
- [ ] Only after chunking + write semantics are proven should durability move from full `DatabaseState` snapshots toward durable physical atoms + realization root authority.

## Pass399 — bounded sparse relation-column chunk materialization + performance gate

- [x] Added bounded physical chunk materialization under the existing factorized relation-column realization rule; semantic relation/column identity and current schema remain unchanged.
- [x] Chunk progress is represented only by sparse native chunk atoms layered over one base realization expression. No dense per-row bitmap and no migration progress journal were introduced.
- [x] A materialized relation-column chunk stores its global `start_row`, so direct reads remain addressed in the same relation-row coordinate space.
- [x] `dependencies()` retains the source/base physical atom while any chunk remains derived; after every chunk is native, the source atom drops automatically from the current GC frontier.
- [x] Hot-range materialization rewrites only the selected bounded range; untouched ranges continue through the same normalized realization rule.
- [x] Release benchmark on 100k rows / 4096-row chunks confirms whole-column sequential and pseudo-random access stay in the same performance class after one chunk is materialized; one chunk costs roughly 0.2–0.4 ms to materialize in this sandbox.

### P399 architecture consequence

Chunk state is physical realization state, not semantic state. There is still one current schema and one semantic relation-column coordinate. A sparse native-chunk overlay is merely a representation refinement of the same factorized rule. GC derives progress from realization reachability; it does not consult a separate migration state machine.

### Next

- [ ] Add the same bounded physical subdivision to factorized entity-field columns, with an explicit carrier/segment coordinate rather than inventing entity-ID-range semantics.
- [ ] Add scan-plan lowering that resolves chunk routing once per segment/range rather than once per scalar read; point lookup remains correct but scan execution should consume physical chunks directly.
- [ ] Develop factorized general relational-query realization/preparation using explicit dependency closures.
- [ ] Add writable B-semantic overlays/lenses before any durability authority migration.

## Pass400 — bounded carrier-segment materialization for factorized entity fields

- [x] Added immutable physical carrier order to the factorized realization path; semantic carrier membership remains a set, but physical segmentation is defined over carrier ordinals, never arithmetic ranges of `EntityId`.
- [x] Added `CarrierSegmentCoordinate { carrier_atom, start_ordinal, len }` and sparse native field chunks under one factorized field realization rule.
- [x] Sparse/holey entity IDs are explicitly covered by regression: a hot entity materializes the bounded carrier segment containing its ordinal, independent of the numeric magnitude/gaps of the ID.
- [x] Current GC frontier keeps the source field atom while any carrier segment remains derived; after full native coverage the old base atom drops automatically from current reachability.
- [x] Added field carrier-range lowering so scans resolve sparse chunk routing per physical segment/range rather than reinterpreting chunk state per scalar.
- [x] Point routing uses native-chunk entity bounds only as a physical lookup index; chunk identity remains the immutable carrier-ordinal segment.
- [x] Release benchmark on 100k sparse entities / 4096-entity segments shows partial random point reads and native hot-segment scans in the same cost class as direct access in stable warm runs; one segment materializes in roughly 0.39–0.44 ms on this sandbox.

### P400 architecture consequence

Entity field chunking and relation-column chunking now share one law: semantic coordinate -> factorized realization rule -> sparse bounded native physical segments. Chunk completion is representation reachability, not semantic migration state. Entity IDs remain semantic identities and are never repurposed as physical packing coordinates.

### Next

- [ ] Develop factorized general relational-query realization/preparation using explicit dependency closures and certified prepared materializations; keep global rewrites fail-closed until exact.
- [ ] Add writable current-schema B overlays/lenses over factorized field/relation realizations so non-injective migration transforms never require inversion.
- [ ] Unify field/relation sparse segment routing behind a small shared physical segment index abstraction if the next hostile pass shows duplication is structural rather than merely two payload shapes.
- [ ] Only after general query preparation + write semantics are proven should durable authority move from full `DatabaseState` snapshots toward physical atoms + realization roots.

## Pass401 — prepared general relational realization + B-native endpoint overlay law

### CLOSED THIS PASS

- [x] General relational migration now has an exact non-fallback preparation path: `RelExpr::scan_relations()` defines the semantic dependency closure; only those factorized source relations are evaluated for preparation.
- [x] `PreparedFactorizedRelation` stores native target-B relation-column atoms plus exact semantic source relations and physical source-atom closure.
- [x] Prepared cutover uses `compose_schema_migration_factorized_with_prepared`; the ordinary compose path stays fail-closed if a general relation is not prepared.
- [x] Cutover no longer performs the general query. On the 100k two-source bag-Union hostile fixture, three warm runs measured 27.471–28.930 ms preparation and 39.769–43.053 us cutover.
- [x] Current-schema B physical writes can install immutable native field/relation chunk overlays after semantic certification. No inverse of the migration transform is required.
- [x] Non-injective field-write hostile coverage proves a B value can replace a derived merge result while the source realization remains unchanged.
- [x] The post-P400 master handoff is now checked into `docs/status/POST_PASS400_MASTER_LEDGER.md` so future passes have an in-repository continuation authority.

### CHOSEN IMPLEMENTATION / R&D LAW

```text
general migration q : A[D] -> B.R

prepare:
    D = scan_relations(q)
    P_R = columnize(q(eval(rho_A | D)))

cutover:
    rho_B.R.column_i = Direct(P_R.column_i)
```

For writes:

```text
certified current-B endpoint
    -> bounded B-native materialization
    -> immutable replacement overlay atom
    -> no inverse A reconstruction
```

`kernel-change` remains semantic conflict/rebase authority. `kernel-realization` only installs already-certified physical endpoints.

### OPEN — IMMEDIATE

- [ ] Compile general preparation into one factorized/streaming `RelExpr` operator DAG so operators that admit streaming do not require temporary row reconstruction. This is an execution specialization of the same algebra, not a fallback/query fork.
- [ ] Define the universal B-native relation delta/endpoint overlay for globally derived relations using existing `kernel-change` / `kernel-query` Γ coordinates. Physical row ordinal is not a sufficient semantic coordinate for arbitrary `Union` / `Distinct` / `Group` / bag results.
- [ ] Decide whether field/relation sparse segment routing has enough structural identity to justify one shared segment-index primitive; do not unify merely because names look similar.
- [ ] After the above: runtime current-world integration so Query/Change/Watch/Auth see only semantic B and lower below that boundary into realization atoms/segments.

### OPEN — DEFERRED / RETURN AFTER CURRENT LINE

- [ ] Durable physical atoms + `RealizationRoot` authority; root switch/crash matrix; single-file compaction over reachable current/historical atoms; encrypted/authenticated atom/root metadata. Do not start before immediate semantic read/write laws close.
- [ ] History retention evolution from generation-level pins to `HistoricalRevisionRoot -> historical RealizationRoot -> reachable PhysicalAtoms` without a second history store.
- [ ] Context: reference/optional-reference patch calculus; relationship patch/mutation calculus; safe create/delete authority; final Database/Context creation/open DX cleanup.
- [ ] Field-granular certified change coordinates so independent field writes do not conflict solely because one physical relation row is shared.
- [ ] DB-owned granular authorization over query/change IR: entity/relation read, field read, traversal, scalar/reference write, relationship mutation, create/delete, history/watch and hosted/session authority. Context shape is not security.
- [ ] Semantic Rules: deterministic regex/`Matches`, richer serialized compositional expressions, entity/model invariants and transaction `require`; no host callbacks.
- [ ] Migration frontend: typed Rust builder/DSL plus Python/TMD/CLI lowering to the same `MigrationModel` / `SchemaMigrationProgram`; diagnostics for missing mappings/preparation.
- [ ] Final Python facade/wheels; Python multi-handle/process lifecycle and error/conflict mapping; .NET/WPF adapter; Studio/CLI; backup/restore/corruption UX; public perf/binary-size budgets.
- [ ] Native Windows secure-memory expansion. Existing AES-256-GCM-SIV / Linux secure-memory product decision remains unchanged; strict third-party constructor-transient elimination is not a mainline blocker.

### SUPERSEDED / DO NOT EXTEND

- [x] Current-world `MixedMigrationRevisionView` / `MixedMigrationPhysicalAuthority` and any Query/Change/Watch/Auth routing by physical A/B epoch.
- [x] Per-value/per-row realization as production storage; it remains correctness-oracle machinery only.
- [x] Generic full-row migration fallback in the live query engine.
- [x] Migration progress bitmap/journal when reachability from realization roots is sufficient.
- [x] Inverse B->A migration as ordinary write or history strategy.
- [x] Authoritative legacy alias chains / `rename_from`; authoritative schema stays current-only, consumer local naming uses `bind`.
- [x] Context omission as authorization; host-language validation callbacks; SDK-side independent query/change/history/watch semantics.

### PERFORMANCE BASELINES TO PRESERVE

- P397 factorized fields: cutover metadata ~46–51 us for 100k values; normalized direct transform native cost class; full-column materialization ~14–16.5 ms.
- P398 factorized relation columns: 100k-row composition ~64–73 us; derived/native scan same cost class; full-column materialization ~3.7–4.1 ms.
- P399 relation chunks: 4096-row hot native chunk scan ~0.96–1.07x direct; bounded materialization roughly 0.17–0.4 ms in prior sandbox measurements.
- P400 sparse entity carrier segments: partial point ~0.94–1.10x direct; hot segment ~0.94–1.07x; one 4096-entity segment ~390–440 us.
- P401 general bag-Union preparation: 100k rows 27.471–28.930 ms; cutover 39.769–43.053 us; semantic deps=2, physical source atoms=2, prepared target atoms=1.

### LEDGER RECONCILIATION — PASS361–PASS400

The PASS361–400 combined reports and POST-PASS400 handoff were reconciled before implementation. The persistent lines match as follows:

- query/projection/group/watch/product DX from P361–P370: major surfaced kernel capabilities are closed; do not create SDK-side fallback engines;
- adaptive transaction/change identity from P371–P373: closed; stable client semantic intent is distinct from realized residual effect and must not be reopened as a new merge engine;
- Semantic Rules P374–P375: base range/length/membership rules closed, regex/general expressions/invariants/`require` still open;
- typed schema/Context P376–P380: physical `Database` vs typed `Context`, semantic projection and scalar patch foundation closed; reference/relationship patches, field-granular changes, auth and final DX still open;
- migration/history P380–P386: semantic migration boundary, cross-epoch history and retained authority laws remain valid;
- P387–P391 mixed-current-world target architecture is superseded, while compact migration program and exact dependency/slice ideas are retained;
- P393–P400 Physical Realization Algebra is the current storage direction and remains the immediate line;
- encryption/secure-memory, bindings and product surfaces remain deferred exactly as carried by the handoff.

### NEXT RECOMMENDED PASS

PASS402: build the factorized/streaming preparation executor and the universal global-relation B delta/endpoint overlay together. The hostile question is whether `RelExpr` dependency/operator structure plus existing Γ relation-change machinery can yield one maintained physical overlay law that supports arbitrary general relation migration writes without unstable row ordinals, source-schema revival, inverse transforms, or a materialize-everything fallback.

## Pass402 — current-B semantic relation endpoint law

### CLOSED THIS PASS

- [x] General migration outputs no longer require a hypothetical writable lens back into source schema A. After cutover the target relation is a first-class current-B semantic coordinate.
- [x] Existing Γ-native `PreparedRelationRewrite` is now the semantic authority consumed by physical realization; no migration-specific conflict/rebase engine was added.
- [x] Prepared relation rewrites expose their bound semantic relation identity so physical realization cannot apply a certificate to another relation.
- [x] `kernel-realization::install_prepared_relation_endpoint` validates the prepared rewrite against the exact currently realized B support and publishes a native-B endpoint; stale prepared support fails closed.
- [x] Bag multiplicity hostile coverage proves the law is not Set-only: one B removal plus one equivalent B insertion produces the exact multiplicity endpoint without touching A.
- [x] Once the native endpoint is installed, old target-relation realization atoms leave the current dependency frontier when not referenced elsewhere.

### CHOSEN IMPLEMENTATION / R&D LAW

```text
migration provenance q : A -> B.R
    matters only while constructing the initial B.R

later current-world write:
    PreparedRelationRewrite_B(R -> R')
        -> physical B realization of R'
        -> never B -> inverse(q) -> A
```

`Union`, `Distinct`, `Group`, Bag, etc. do not create write routers. The exact B relation rewrite is the universal semantic principle.

### PERFORMANCE HOSTILE / REJECTED PRODUCTION LOWERING

- [x] Full native endpoint detachment is retained as a correctness/reference lowering, not accepted as the ordinary hot write path.
- [x] 100k-row bag-Union + one-row write, three warm release runs: **92.918 ms / 98.899 ms / 94.569 ms**.
- [x] Therefore O(|relation|) endpoint reconstruction/materialization for a one-row write is an explicit bottleneck, not hidden behind a fallback.

### OPEN — IMMEDIATE

- [ ] R&D a bounded immutable **B-native relation delta overlay** over Γ canonical relation classes / exact Bag multiplicity. It must consume the same `PreparedRelationRewrite`, not invent new semantics.
- [ ] Give that overlay an execution lowering for scans/points and a compaction law back to native column segments; bounded writes must not require O(|relation|) row reconstruction.
- [ ] Continue factorized/streaming `RelExpr` preparation so general migration preparation stops reconstructing temporary row models where an operator DAG can execute column/segment-wise.
- [ ] Only after bounded general-relation writes + preparation execution are stable: runtime current-world integration for Query/Change/Watch/Auth.

### OPEN — DEFERRED / RETURN AFTER CURRENT LINE

- [ ] Durable physical atoms + `RealizationRoot` authority, crash/root-switch matrix, single-file compaction and encrypted/authenticated atom/root metadata.
- [ ] History retention evolution to `HistoricalRevisionRoot -> RealizationRoot -> PhysicalAtoms` without a second history store.
- [ ] Context reference/optional-reference patches, relationship mutation calculus, safe create/delete authority and final Database/Context creation/open DX.
- [ ] Field-granular certified change coordinates for independent partial writes.
- [ ] DB-owned granular authorization over query/change IR; Context shape is not security.
- [ ] Deterministic regex/`Matches`, richer serialized Semantic Rules, model invariants and transaction `require`; no host callbacks.
- [ ] Migration frontend DSL/diagnostics shared by Rust/Python/TMD/CLI.
- [ ] Final Python/.NET/Studio surfaces, backup/restore/corruption UX, public perf/binary-size budgets, native Windows secure-memory expansion.

### SUPERSEDED / DO NOT EXTEND

- [x] Treating general migration output as a permanently writable view that must push writes through `Union`/`Distinct`/`Group` back into A.
- [x] Source-row provenance/inverse migration as a prerequisite for current-B relation writes.
- [x] Current-world A/B routing, SQL writable-view fallback, per-row production realization, migration progress journals, Context-as-auth and host callback validation remain superseded as carried from P401.

### PERFORMANCE BASELINES TO PRESERVE

- P397 factorized fields: ~46–51 us structural cutover metadata; normalized hot transform native cost class; full 100k materialization ~14–16.5 ms.
- P398 relation columns: 100k composition ~64–73 us; derived/native scan same cost class; full-column materialization ~3.7–4.1 ms.
- P399 relation chunks: 4096-row hot chunk scan ~0.96–1.07x direct.
- P400 sparse entity carrier segments: partial point ~0.94–1.10x direct; hot segment ~0.94–1.07x; one segment ~390–440 us.
- P401 general bag-Union preparation: 100k rows 27.471–28.930 ms; cutover 39.769–43.053 us.
- P402 reference current-B full detach: 100k rows / one-row delta **92.918–98.899 ms** — explicitly rejected as production hot-write target.

### LEDGER CARRY RULE

All unresolved Context/Auth/Semantic Rules/history/bindings/durability/product lines above remain mandatory carry-forward items. A future PASS may remove one only by recording `CLOSED`, `SUPERSEDED`, or `REJECTED` with the concrete replacement/reason.

### NEXT RECOMMENDED PASS

PASS403: derive the bounded B-native relation-delta physical algebra. The target is one immutable overlay law for Set Γ-classes and exact Bag multiplicity, with no source-schema provenance, no unstable semantic row ordinal, bounded write cost, exact scan lowering, and compaction back to native factorized columns.

## Pass403 — bounded current-B relation delta physical algebra

### CLOSED THIS PASS

- [x] PASS402 O(|R|) full endpoint detachment is no longer the intended hot-write lowering.
- [x] General-relation targets can prepare an exact Γ-aware write index before cutover and then accept current-B `PreparedRelationRewrite` deltas without reconstructing the full relation.
- [x] Set removals route by pinned Γ canonical class; hostile case-insensitive text coverage removes `ALPHA` from physical `Alpha` exactly.
- [x] Bag writes preserve exact multiplicity through the same overlay law.
- [x] Insertions become immutable B-native `RelationRows` delta atoms; removals/order changes are persistent physical routing metadata, not semantic row identity.
- [x] Root snapshots use persistent COW maps/vectors; root clone is bounded rather than O(|R|).
- [x] Range scan lowering consumes contiguous untouched base runs directly and only dispatches overlay occurrences separately.
- [x] Overlay compaction returns to native factorized columns and releases obsolete current-root delta/old-target atoms by reachability.
- [x] Missing write preparation fails closed; there is no hidden full-detach fallback.
- [x] Ordinary P401 general-relation preparation is not forced to build the write index.

### CHOSEN IMPLEMENTATION / R&D LAW

```text
general migration preparation
    -> native target columns
    -> optional exact write-overlay preparation before cutover

current B write
    PreparedRelationRewrite_B
        -> exact RelationBaseWitness
        -> persistent Γ occurrence routing update
        -> immutable delta atom
        -> RealizationRoot'

compaction
    base + overlays == native endpoint
```

### PERFORMANCE BASELINES TO PRESERVE

- P401 ordinary general relation preparation remains ~31.7–33.5 ms in current repeated runs; cutover ~44.6–46.9 us.
- P402 rejected full detach: ~92.9–98.9 ms per one-row write over 100k rows.
- P403 100k bounded write after write-index preparation: prepared semantic rewrite ~5.6–6.5 us; root clone ~3.5–3.8 us; overlay install ~22.8–26.5 us.
- P403 scan with one appended overlay occurrence remains native cost class in stable runs (~0.92–1.01x); one 1.69x noisy outlier was observed and is not accepted as the stable conclusion.
- P403 one-time write-index preparation is still ~294–335 ms for 100k rows and is the immediate optimization target.

### OPEN — IMMEDIATE

- [ ] R&D one shared Γ occurrence/support structure so `RelationBaseWitness` and physical overlay routing do not separately build canonical indexes; preserve bounded writes while cutting the ~294–335 ms write-index preparation cost.
- [ ] Extend bounded overlay preparation beyond prepared general-relation targets to the universal current relation physical owner without forcing read-only/native relations to pay unused metadata.
- [ ] Hostile-test repeated mixed removals/inserts, compaction thresholds, multiple overlay atoms and long-lived snapshot sharing.
- [ ] Continue factorized/streaming `RelExpr` preparation so global migration preparation avoids temporary row models where the operator DAG can execute column/segment-wise.
- [ ] Then connect realization to runtime current-world Query/Change/Watch/Auth execution.

### OPEN — DEFERRED / RETURN AFTER CURRENT LINE

- [ ] Durable physical atoms + `RealizationRoot` authority, crash/root-switch matrix, single-file compaction and encrypted/authenticated atom/root metadata.
- [ ] History retention evolution to `HistoricalRevisionRoot -> RealizationRoot -> PhysicalAtoms` without a second history store.
- [ ] Context reference/optional-reference patches, relationship mutation calculus, safe create/delete authority and final Database/Context creation/open DX.
- [ ] Field-granular certified change coordinates for independent partial writes.
- [ ] DB-owned granular authorization over query/change IR; Context shape is not security.
- [ ] Deterministic regex/`Matches`, richer serialized Semantic Rules, model invariants and transaction `require`; no host callbacks.
- [ ] Migration frontend DSL/diagnostics shared by Rust/Python/TMD/CLI.
- [ ] Final Python/.NET/Studio surfaces, backup/restore/corruption UX, public perf/binary-size budgets, native Windows secure-memory expansion.

### SUPERSEDED / DO NOT EXTEND

- [x] O(|R|) full native endpoint detachment as ordinary current-B hot write.
- [x] Mutable/std collection overlay snapshots that clone O(|R|); physical overlay metadata is persistent COW.
- [x] Source-A provenance/inverse migration, operator-specific writable-view routing, current A/B routing and SQL fallback remain superseded.

### LEDGER CARRY RULE

All unresolved Context/Auth/Semantic Rules/history/bindings/durability/product lines remain mandatory carry-forward items. A line disappears only as `CLOSED`, `SUPERSEDED`, or `REJECTED` with the concrete replacement/reason.

### NEXT RECOMMENDED PASS

PASS404: unify semantic Γ support and physical occurrence routing into one reusable exact witness/index, removing duplicated canonical preparation while preserving the P403 ~tens-of-microseconds write path. Then stress repeated overlays/compaction before runtime integration.

## Pass404 — shared Γ occurrence witness + sparse physical routing

### CLOSED THIS PASS

- [x] `RelationBaseWitness` is now the single exact Γ occurrence/support authority for bounded relation writes: canonical class -> persistent stable occurrence handles. Physical realization no longer builds a second canonical class index.
- [x] Prepared relation rewrites carry the exact removed/inserted `StableRowHandle`s derived by that witness; physical overlay installation consumes those handles directly and does not re-canonicalize the touched rows.
- [x] The next relation witness is derived from the already-prepared structural transition, so physical install does not independently replay/canonicalize the semantic delta.
- [x] Dense O(|R|) physical handle->position/base maps were rejected. Initial identity routing now needs only the persistent logical handle vector; a sparse override map records only handles displaced by later mutations.
- [x] Base physical row ordinal remains realization metadata. `StableRowHandle` is storage identity; it is not promoted to a semantic relation coordinate.
- [x] Initial write preparation is columnar: factorized relation columns feed `RelationBaseWitness::build_columnar` directly, avoiding temporary `Vec<Row>` reconstruction and one heap-backed row allocation per relation occurrence.
- [x] Γ equivalence modules are compiled once per relation witness and reused for canonical-key construction / later bounded deltas rather than being resolved per value.
- [x] Initial canonical occurrence construction uses sort/group + an owned balanced persistent-map bulk builder, avoiding BTree node churn and the previous second clone of every canonical key during persistent-map construction.
- [x] Compaction may rebind physical occurrence handles to the newly compacted native row order; previously prepared rewrites then fail closed against the new witness authority rather than treating physical ordinals as semantic identity.

### CHOSEN IMPLEMENTATION / R&D LAW

```text
one exact RelationBaseWitness
    canonical Γ class
        -> persistent StableRowHandle occurrences

PreparedRelationRewrite_B
    -> semantic delta validation / Γ classification once
    -> exact removed + inserted occurrence handles
    -> next witness root

physical B overlay
    -> consumes those handles directly
    -> sparse logical-position overrides only for displaced occurrences
    -> immutable inserted-row atoms
```

The semantic and physical layers share evidence, not authority: `kernel-query` creates the exact occurrence transition; `kernel-realization` only maps the certified storage handles into physical rows/atoms.

### PERFORMANCE BASELINES TO PRESERVE

100k-row Bag `Union`, one inserted B row, final warm release runs after the full P404 refactor:

- write-overlay preparation: **183.000 / 192.138 / 213.294 ms** versus P403 **~294–335 ms**;
- prepared semantic rewrite: **~5.6–5.8 us** in the final runs;
- realization-root clone: **~2.3–2.4 us**;
- physical overlay install: **~11.9–12.7 us**;
- full 100k scan with one overlay occurrence: **~0.98–1.09x** direct/native in the final three runs.

P404 therefore removes roughly one-third to two-fifths of the one-time P403 write-preparation cost while also reducing the hot overlay install from P403 ~22.8–26.5 us to ~12 us. The remaining ~0.18–0.21 s write-preparation cost is still O(data) and remains an optimization target; it is not treated as free.

### OPEN — IMMEDIATE

- [ ] Hostile-test long repeated mixed remove/insert sequences, multiple immutable delta atoms, old snapshot sharing, and compaction/rebinding thresholds now that occurrence handles are the shared witness substrate.
- [ ] Extend optional bounded-write witness preparation from prepared general-migration targets to the universal current-relation physical owner without forcing read-only/native relations to pay O(data) metadata construction.
- [ ] Continue reducing one-time witness preparation: investigate whether canonical occurrence construction can reuse maintained/query-side Γ support already present at runtime, or be produced during general relation preparation without a second full pass, while retaining one authority and no hot-path fallback.
- [ ] Continue factorized/streaming `RelExpr` preparation so general migration operators do not materialize temporary row models where column/segment execution is available.
- [ ] After the above hostile coverage: runtime current-world integration where Query/Change/Watch/Auth see only semantic B and realization owns the physical occurrence routing below that boundary.

### OPEN — DEFERRED / RETURN AFTER CURRENT LINE

- [ ] Durable physical atoms + durable `RealizationRoot`, crash/root-switch matrix, single-file compaction and encrypted/authenticated atom/root metadata.
- [ ] History retention evolution to `HistoricalRevisionRoot -> RealizationRoot -> PhysicalAtoms` without a second history store.
- [ ] Context reference/optional-reference patches, relationship mutation calculus, safe create/delete authority and final Database/Context creation/open DX.
- [ ] Field-granular certified change coordinates for independent partial writes.
- [ ] DB-owned granular authorization over query/change/history/watch/hosted-session coordinates; Context shape is not security.
- [ ] Deterministic regex/`Matches`, richer serialized Semantic Rules, entity/model invariants and transaction `require`; no host callbacks.
- [ ] Migration frontend Rust/Python/TMD/CLI DSL + diagnostics over the same runtime/kernel migration model.
- [ ] Final Python/.NET/Studio surfaces, backup/restore/corruption UX, public perf/binary-size budgets, native Windows secure-memory expansion.

### SUPERSEDED / DO NOT EXTEND

- [x] Separate physical Γ canonical class index beside `RelationBaseWitness`.
- [x] Dense identity-time handle->position/base routing maps for every relation row.
- [x] Temporary row-wise reconstruction solely to prepare the bounded-write witness when native factorized columns already exist.
- [x] Re-canonicalizing a prepared relation delta again inside physical overlay installation.
- [x] P402 O(|R|) full endpoint detachment as ordinary hot-write lowering, source-A provenance/inverse migration, operator-specific writable-view routing, current A/B routing and SQL fallback remain superseded.

### LEDGER CARRY RULE

All unresolved Context/Auth/Semantic Rules/history/bindings/durability/product lines above remain mandatory carry-forward items. A line may disappear only as `CLOSED`, `SUPERSEDED`, or `REJECTED` with the concrete replacement/reason.

### NEXT RECOMMENDED PASS

PASS405: hostile repeated-overlay lifecycle on the shared occurrence witness: mixed Set/Bag removals+insertions across many immutable delta atoms, persistent old-root snapshots, compaction/rebinding, stale prepared rewrite rejection and a quantitative compaction threshold. In parallel, inspect whether runtime-maintained Γ support can seed the witness directly so optional write preparation no longer needs a separate O(data) canonical pass. Only after that should the realization layer be connected to the runtime current-world execution path.

## Pass405 — repeated-overlay lifecycle + cost-based compaction law

### CLOSED THIS PASS

- [x] Hostile repeated Bag lifecycle now covers 48 exact remove+insert transitions over the same current-B relation, stale prepared-rewrite rejection after every semantic mutation, six compaction/rebinding cycles, and long-lived cloned roots retained across later writes/compactions.
- [x] Repeated Γ-Set lifecycle now covers multiple case-insensitive removals (`ALPHA`, `BETA`), multiple immutable delta atoms, compaction, and preservation of an old pre-compaction snapshot.
- [x] Physical overlay state exposes exact `RelationDeltaOverlayStats`: base/live rows, live inserted rows, displaced logical positions, and currently reachable immutable delta atoms. These are representation metrics only; they do not become migration progress or database semantics.
- [x] Compaction is no longer modeled as a fixed write-count threshold. The selected law is amortized physical cost: compact only when expected future work saved by native layout exceeds compaction cost. For full scans, the measurable decision inequality is `H * (C_overlay_scan - C_native_scan) >= C_compact`, where `H` is expected remaining full scans before the overlay would otherwise disappear/change materially.
- [x] A maintained-query support reuse prototype was hostile-tested and REJECTED rather than shipped: translating existing `CanonicalRowPositionIndex` buckets into a fresh handle witness still cloned/remapped O(|R|) canonical support. On 100k rows it measured ~348.6 ms versus ~305.9 ms for direct witness recanonicalization in that prototype. Reuse must share the occurrence authority structurally, not translate one O(data) index into another.

### CHOSEN IMPLEMENTATION / R&D LAW

```text
semantic write sequence
    PreparedRelationRewrite_B^0
    PreparedRelationRewrite_B^1
    ...
        -> one persistent RelationBaseWitness lineage
        -> immutable delta atoms + sparse physical routing
        -> old RealizationRoot snapshots remain exact

physical maintenance
    observe overlay metrics
    estimate remaining workload H
    compact iff
        H * (C_overlay - C_native) >= C_compact

compaction is representation economics,
not semantic progress and not a fixed "every N writes" rule.
```

### PERFORMANCE / HOSTILE BASELINES

100k-row Bag relation, repeated one-row replacements, 8 full scans per sample; three final warm release runs:

- depth 1: **1.061–1.141x** native scan;
- depth 16: **1.241–1.273x**;
- depth 128: two stable runs **1.657–1.694x**, one noisy **2.625x** outlier;
- depth 1024: **1.770–1.846x**;
- depth-1024 compaction: **269.9 / 274.2 / 270.8 ms**;
- measured full-scan break-even at depth 1024: about **172–192 future full scans** on this fixture/machine, using the amortization law above.

The exact numeric threshold is not a product constant. Runtime integration must measure/estimate the relevant workload and use the cost law, not hard-code 1024 overlays or any benchmark-specific ratio.

### OPEN — IMMEDIATE

- [ ] Replace the rejected maintained-index translation with a structural shared occurrence authority: maintained Scan / current physical relation and `RelationBaseWitness` should share the same canonical-class -> stable-occurrence root from construction/attachment, rather than converting positions to handles after the fact.
- [ ] Hostile point/range workloads as well as full scans and derive a multi-workload compaction estimator; full-scan economics alone are insufficient for write-heavy or point-heavy databases.
- [ ] Extend the optional bounded-write witness/overlay to the universal current-relation physical owner without forcing read-only/native relations to pay O(data) write metadata.
- [ ] Continue factorized/streaming `RelExpr` preparation so general migration operators avoid temporary row materialization where column/segment execution exists.
- [ ] Then runtime current-world integration: Query/Change/Watch/Auth remain semantic-B; physical occurrence routing/maintenance stay below that boundary.

### OPEN — DEFERRED / RETURN AFTER CURRENT LINE

- [ ] Durable physical atoms + durable `RealizationRoot`, crash/root-switch matrix, single-file compaction and encrypted/authenticated atom/root metadata.
- [ ] History retention evolution to `HistoricalRevisionRoot -> RealizationRoot -> PhysicalAtoms`, without a second history store.
- [ ] Context reference/optional-reference patches, relationship mutation calculus, safe create/delete and final Database/Context DX.
- [ ] Field-granular certified change coordinates.
- [ ] DB-owned granular authorization over query/change/history/watch/hosted-session coordinates.
- [ ] Deterministic regex/`Matches`, richer serialized Semantic Rules, entity/model invariants and transaction `require`.
- [ ] Migration frontend Rust/Python/TMD/CLI DSL + diagnostics over the same runtime migration model.
- [ ] Final Python/.NET/Studio surfaces, backup/restore/corruption UX, public perf/binary-size budgets, native Windows secure-memory expansion.

### SUPERSEDED / DO NOT EXTEND

- [x] Fixed overlay-count compaction thresholds as semantic/product policy.
- [x] Translating a maintained `CanonicalRowPositionIndex` into a second O(data) handle witness and calling that "reuse".
- [x] Separate physical Γ authority, dense handle maps, physical recanonicalization of prepared deltas, P402 full hot detach, source-A inverse/provenance routing, SQL fallback, migration-progress journals, Context-as-auth and host callback validation remain superseded.

### PERFORMANCE BASELINES TO PRESERVE

- P403 bounded install ~22.8–26.5 us; P404 ~11.9–12.7 us; root clone ~2.3–2.4 us.
- P404 one-time write preparation ~183–213 ms / 100k; still O(data), still open.
- P405 confirms scan cost grows with live overlay fragmentation; one overlay remains native-class while ~1% replaced rows / 1024 delta atoms reached ~1.77–1.85x full-scan cost in stable runs.
- Compaction itself is expensive (~270 ms / 100k at depth 1024 here), therefore eager/fixed-N compaction is explicitly rejected.

### LEDGER CARRY RULE

All unresolved Context/Auth/Semantic Rules/history/bindings/durability/product lines remain mandatory carry-forward items. A line disappears only as `CLOSED`, `SUPERSEDED`, or `REJECTED` with the concrete replacement/reason.

### NEXT RECOMMENDED PASS

PASS406: make the shared Γ occurrence witness a real runtime/current-relation owner rather than an optional migration-only preparation artifact. Start by hostile-designing one canonical occurrence directory that maintained Scan storage attachment and physical realization can both share directly (O(1)/persistent-root handoff, no O(data) translation). In the same pass benchmark point/range reads under overlay depth so the compaction estimator covers actual mixed workloads before runtime integration is declared closed.

## PASS406 — shared current-relation Γ witness handoff + mixed-read overlay cost

### CLOSED THIS PASS

- Maintained `Scan` can bind the exact already-built `RelationBaseWitness` owned by the current physical relation together with identity-domain storage handles. The witness clone shares the same persistent Γ-occurrence root; no `CanonicalRowPositionIndex -> StableRowHandle` Γ translation or row re-canonicalization is performed.
- `MaterializedRelPlanState::storage_relation_base_witness` returns that shared authority in O(1) persistent-root clone time when every source occurrence agrees on the same base.
- Storage-resolved transitions advance and retain the shared witness only when removed/inserted handles exactly match the witness transition. Generic or incompatible mutation drops sharing rather than fabricating a second Γ authority.
- Forged/non-identity handle domains fail closed for the shared handoff contract.
- Overlay-depth hostile now measures full-scan, random-point and 4096-row range workloads. Compaction policy is explicitly workload-weighted; full-scan-only thresholds are insufficient.

### OPEN — IMMEDIATE

1. Move the shared witness from an optional attach contract into the universal runtime/current-relation owner so query/watch and realization receive the same authority by construction rather than orchestration convention.
2. Eliminate the remaining one-time O(data) witness build by fusing occurrence-root construction with the physical/current relation creation path; do not translate a maintained position index after the fact.
3. Extend the compaction estimator to measured workload weights: point, bounded range and full scans, plus write/install cost and compaction cost.
4. Continue factorized/streaming general `RelExpr` preparation so global migration preparation avoids temporary row materialization where column/segment execution exists.
5. Then connect runtime current-world Query/Change/Watch/Auth to physical realization beneath the semantic-B boundary.

### OPEN — DEFERRED / RETURN AFTER CURRENT LINE

- durable physical atoms + durable `RealizationRoot`, crash/root-switch matrix, single-file compaction and encrypted/authenticated atom/root metadata;
- history retention evolution to `HistoricalRevisionRoot -> RealizationRoot -> PhysicalAtoms`, without a second history store;
- Context reference/optional-reference patches, relationship mutation calculus, safe create/delete and final Database/Context DX;
- field-granular certified change coordinates;
- DB-owned granular authorization over query/change/history/watch/hosted-session coordinates;
- deterministic regex/`Matches`, richer serialized Semantic Rules, entity/model invariants and transaction `require`;
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics over the same runtime migration model;
- final Python/.NET/Studio surfaces, backup/restore/corruption UX, public perf/binary-size budgets, native Windows secure-memory expansion.

### SUPERSEDED / DO NOT EXTEND

- O(data) maintained canonical-position -> handle-witness translation;
- a second physical Γ authority;
- full-scan-only or fixed-N compaction policy;
- source-A inverse/provenance routing, operator-specific writable views, SQL fallback, migration progress journals, Context-as-auth and host callback validation.

### PERFORMANCE BASELINES TO PRESERVE

- P404 bounded install ~11.9–12.7 us; root clone ~2.3–2.4 us.
- P404/P405 one-time optional witness preparation remains O(data) (~0.18–0.21 s best accepted line at 100k) and is still an optimization target, not hidden as free.
- P406 100k depth-1024 warm runs: full scan ~1.65–1.76x native; random point ~1.19–1.73x; 4096-row range ~1.66–1.72x. Point results are noisier and must not be collapsed into one fixed factor.
- P406 depth-1024 compaction warm runs ~229–245 ms; full-scan-only break-even ~161–191 future scans on this fixture, not a product constant.

### LEDGER CARRY RULE

Every successor PASS must keep all unresolved Context/Auth/Semantic Rules/history/bindings/durability/product lines visible. A line disappears only as `CLOSED`, `SUPERSEDED`, or `REJECTED` with its concrete replacement/reason.

## PASS407 — construction-time current-relation Γ authority

### CLOSED THIS PASS

- General relational physical preparation no longer has an optional second `prepare_write_overlay()` phase. `PreparedFactorizedRelation` is structurally write-capable at construction: an exact `RelationBaseWitness` and an empty bounded `RelationDeltaOverlay` are mandatory state, not optional orchestration.
- The target `FactorizedRealizationRoot` receives a persistent clone of that exact occurrence root at semantic cutover. Hostile coverage proves the prepared authority and current root share the same Γ-occurrence persistent root.
- Bounded current-B writes therefore retain the P403/P404 hot law without any post-cutover or first-write O(data) witness preparation. On the 100k fixture the separate witness-preparation stage is exactly absent; semantic rewrite preparation stays ~5.4 us, root clone ~2.5–2.9 us, physical overlay install ~11.9–13.2 us, and depth-1 scan remains native cost class.
- The old historical P403 rule that general migration may omit the write index is SUPERSEDED for the current architecture. A prepared/current general relation now has one Γ occurrence authority by construction.

### HOSTILE RESULT / BOUNDARY

- Construction-time ownership removes the duplicate phase but does **not** make Γ quotient construction O(1). Warm 100k `Union` preparation with the mandatory current-relation witness measured ~225–267 ms; cutover remained ~54–80 us.
- This is comparable to paying the former general preparation plus one-time witness construction, so PASS407 does not claim the O(data) mathematics disappeared. Merely moving `RelationBaseWitness::build` into realization would be an accounting trick if described as a performance win.
- The remaining payer is semantic: arbitrary `RelExpr` output must acquire an exact Γ occurrence partition. The clean next R&D law is for relational execution/materialization itself to emit `RelationValue + certified occurrence support` compositionally, so realization consumes already-proved output support instead of canonicalizing the completed rows again.

### OPEN — IMMEDIATE

1. R&D one compositional `RelExpr` materialization result carrying exact Γ occurrence/support evidence together with the output relation. Reuse maintained Set/Bag/Distinct/Group/Join support laws; do not add operator-specific migration fallbacks.
2. Make physical general-relation construction consume that execution certificate directly, eliminating repeated output canonicalization while preserving one current semantic B world.
3. Wire the construction-time authority through runtime current-relation ownership so Query/Watch/Change receive it automatically; keep exact row/storage binding fail-closed rather than trusting a mismatched model by convention.
4. Implement/measure the workload-weighted compaction estimator using point/range/full-scan observations plus write/install and compaction cost.
5. Continue factorized/streaming general `RelExpr` preparation so global migration preparation avoids temporary row materialization where column/segment execution exists.
6. Then connect runtime current-world Query/Change/Watch/Auth to physical realization beneath the semantic-B boundary.

### OPEN — DEFERRED / RETURN AFTER CURRENT LINE

- durable physical atoms + durable `RealizationRoot`, crash/root-switch matrix, single-file compaction and encrypted/authenticated atom/root metadata;
- history retention evolution to `HistoricalRevisionRoot -> RealizationRoot -> PhysicalAtoms`, without a second history store;
- Context reference/optional-reference patches, relationship mutation calculus, safe create/delete and final Database/Context DX;
- field-granular certified change coordinates;
- DB-owned granular authorization over query/change/history/watch/hosted-session coordinates;
- deterministic regex/`Matches`, richer serialized Semantic Rules, entity/model invariants and transaction `require`;
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics over the same runtime migration model;
- final Python/.NET/Studio surfaces, backup/restore/corruption UX, public perf/binary-size budgets, native Windows secure-memory expansion.

### SUPERSEDED / DO NOT EXTEND

- optional `PreparedFactorizedRelation::prepare_write_overlay` as a separate current-B write-index preparation phase;
- O(data) maintained canonical-position -> handle-witness translation;
- a second physical Γ authority;
- fixed-N, fixed-depth or full-scan-only compaction policy;
- source-A inverse/provenance routing, operator-specific writable views, SQL fallback, migration progress journals, Context-as-auth and host callback validation.

### PERFORMANCE BASELINES TO PRESERVE

- P407 mandatory current-relation construction, 100k `Union`: ~225–267 ms warm; semantic cutover ~54–80 us. Treat the O(data) quotient work as an explicit remaining payer.
- P407 bounded current-B hot path after construction: semantic prepare ~5.4 us; root clone ~2.5–2.9 us; overlay install ~11.9–13.2 us; one-overlay scan ~0.98–1.10x direct in the observed runs.
- P406 depth-1024 warm ranges remain: full scan ~1.65–1.76x native; random point ~1.19–1.73x; 4096-row range ~1.66–1.72x; compaction ~229–245 ms on the fixture.

### LEDGER CARRY RULE

Every successor PASS must keep all unresolved Context/Auth/Semantic Rules/history/bindings/durability/product lines visible. A line disappears only as `CLOSED`, `SUPERSEDED`, or `REJECTED` with its concrete replacement/reason.

### NEXT RECOMMENDED PASS

PASS408: move the Γ occurrence certificate upstream into exact relational execution. Hostile-design a single compositional materialization result that can cover Bag/Set/Distinct/Group/Join/Union without migration-specific routing, then prove that physical general-relation construction can consume it without a second canonical pass. Do not touch durability until this semantic/execution boundary is stable.

## PASS408 — execution-issued Γ occurrence certificate (Set-Union first law)

### CLOSED THIS PASS

- Exact relational execution now has a first-class `RelationOccurrenceCertificate`: one persistent `canonical Γ class -> StableRowHandle[]` root emitted together with result rows rather than reconstructed later by realization.
- `RelationBaseWitness::from_occurrence_certificate` adopts that persistent root without any row canonicalization. Cross-revision adoption is not tied to literal `SemanticContext` identity; it is accepted only when the source-output and target-relation compiled column equivalence laws are exactly equal. Schema extension therefore remains legal while a changed Γ law fails closed.
- `Set Union` is the first operator implementation of the law. The operator already must compute the Γ quotient for union semantics, so the same canonical keys now also become the physical/current occurrence certificate. No second output canonical pass is performed.
- General relation preparation consumes the certificate for `Set Union` and constructs the mandatory current-B `RelationBaseWitness` from it. The physical root still receives the same construction-time authority established by P407.
- Hostile case-insensitive Set coverage proves that `Alpha` and `ALPHA` collapse to one output Γ class and that the resulting execution certificate and adopted relation witness share the exact persistent occurrence root.

### CHOSEN IMPLEMENTATION / R&D LAW

```text
operator already computes exact Γ quotient/support
    -> emit output rows + RelationOccurrenceCertificate in that same execution
    -> verify target compiled Γ laws are identical
    -> adopt persistent occurrence root as RelationBaseWitness
    -> no post-execution recanonicalization

operator does not otherwise need Γ quotient (for example plain Bag Union)
    -> there is no quotient work to "reuse"
    -> one exact occurrence construction is still mathematically required for writable current-B authority
```

This distinction is algebraic, not a SQL/operator-name fallback. The certificate is reusable evidence only where execution has actually proved the required quotient/support.

### HOSTILE / PERFORMANCE RESULT

100k distinct-row **Set Union**, release, three warm comparison runs against the previous exact path (`evaluate` followed by `RelationBaseWitness::build`):

- baseline: **261.5–270.9 ms**;
- execution-issued certificate: **73.1–78.7 ms**;
- ratio: **0.272–0.291x** (about **3.4–3.7x faster**).

A forced certificate prototype for 100k **Bag Union** measured roughly **295–318 ms** and was REJECTED. Ordinary Bag Union does not compute a quotient, so that prototype merely moved/added the required canonical support construction and regressed the accepted P407 line. Bag remains on the single canonical-pass construction law until execution has stronger support evidence to reuse.

### OPEN — IMMEDIATE

1. Generalize the execution-issued certificate law to the operators that already compute exact quotient/support internally: `Distinct`, Set projection, `Group`, Set `Difference`/`AntiJoin`, Set-producing `Join`, and maintained execution barriers where the same evidence already exists.
2. Derive a clean Bag occurrence-certificate construction from existing maintained multiplicity state where available; do not force quotient work into operators that otherwise do not need it merely for symmetry.
3. Remove the remaining realization-side operator pattern once all general relation result classes can supply the common certificate law directly. Until then unsupported certificate reuse stays explicit rather than hidden behind a pretend generic fast path.
4. Wire execution-issued occurrence authority through runtime current-relation ownership so Query/Watch/Change consume the same root automatically.
5. Continue workload-weighted compaction estimation and factorized/streaming global `RelExpr` preparation.

### OPEN — DEFERRED / RETURN AFTER CURRENT LINE

- durable physical atoms + durable `RealizationRoot`, crash/root-switch matrix, single-file compaction and encrypted/authenticated atom/root metadata;
- history retention evolution to `HistoricalRevisionRoot -> RealizationRoot -> PhysicalAtoms`, without a second history store;
- Context reference/optional-reference patches, relationship mutation calculus, safe create/delete and final Database/Context DX;
- field-granular certified change coordinates;
- DB-owned granular authorization over query/change/history/watch/hosted-session coordinates;
- deterministic regex/`Matches`, richer serialized Semantic Rules, entity/model invariants and transaction `require`;
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics over the same runtime migration model;
- final Python/.NET/Studio surfaces, backup/restore/corruption UX, public perf/binary-size budgets, native Windows secure-memory expansion.

### SUPERSEDED / REJECTED / DO NOT EXTEND

- REJECTED: forcing execution-issued quotient construction into plain Bag Union solely to make the API look uniform when execution otherwise has no quotient work to reuse.
- SUPERSEDED for Set Union: `evaluate complete rows -> RelationBaseWitness::build -> canonicalize the same output again`.
- Still superseded: optional write-overlay preparation, maintained-position -> handle translation, second physical Γ authority, fixed-N/depth compaction, source-A inverse/provenance routing, SQL fallback, migration progress journals, Context-as-auth and host callback validation.

### PERFORMANCE BASELINES TO PRESERVE

- P408 Set Union 100k execution certificate: **73.1–78.7 ms** versus **261.5–270.9 ms** old two-pass path.
- P407 Bag/general construction accepted line remains ~**225–267 ms** rather than the rejected forced-certificate ~295–318 ms prototype.
- P407 hot current-B write path remains semantic prepare ~5.4 us, root clone ~2.5–2.9 us, install ~11.9–13.2 us.
- P406 depth-1024 mixed-read and compaction ranges remain regression gates.

### LEDGER CARRY RULE

Every successor PASS must carry all unresolved Context/Auth/Semantic Rules/history/bindings/durability/product lines. A line disappears only as `CLOSED`, `SUPERSEDED`, or `REJECTED` with the concrete replacement/reason.

### NEXT RECOMMENDED PASS

PASS409: extend `RelationOccurrenceCertificate` composition to `Distinct`/Set projection/Group first, because those operators already own quotient/group canonical keys and therefore offer the same genuine reuse opportunity as Set Union. Then remove the temporary realization-side Set-Union specialization once the certificate is a general exact-execution result rather than an operator-specific fast path.

## PASS409 — quotient-native occurrence certificates beyond Union

### CLOSED THIS PASS

- `PreparedRelExpr` now owns the execution-certificate capability decision. `kernel-realization` no longer pattern-matches `RelExpr::Union`; it asks the prepared exact query whether its algebra natively emits a reusable Γ-occurrence certificate.
- `Distinct` emits its certificate from the exact canonical keys already used to form its quotient. The certified path does not canonicalize output rows a second time.
- Set `Project` emits its certificate from the exact projected quotient keys already required by Set projection semantics. Bag projection explicitly does not claim this capability.
- The exact result/certificate adoption law from P408 is unchanged: target compiled Γ laws must match before `RelationBaseWitness` adopts the persistent occurrence root.
- Hostile regression covers case-insensitive `Distinct` and Set projection and proves the adopted witness shares the execution-issued persistent occurrence root.

### HOSTILE RESULT / REJECTED THIS PASS

- A `Group` certificate prototype was implemented and benchmarked, then removed. Even after reusing the canonical group key rather than re-canonicalizing it, 100k/50k-group `Group(count)` measured about **1.50–1.54x** the accepted baseline. The additional occurrence-root construction dominates the cheap second witness pass on this fixture.
- Therefore `Group` is **REJECTED for direct execution-issued certificate publication in its current exact executor**. Revisit only if maintained-group state can hand off an already-owned persistent support/occurrence root rather than constructing another one for symmetry.
- Cheap `I64Exact` 100k hostile after warm-up kept accepted operators in the same cost class: Set projection about **0.97–1.06x**, `Distinct` about **0.94–1.00x**. No speedup claim is made for these cheap-equivalence runs; the architectural gain is removal of a second semantic canonicalization without a stable regression.

### OPEN — IMMEDIATE

1. Extend certificate reuse only where the operator already owns sufficient exact support: hostile Set `Difference` first, then inspect `AntiJoin`/Set-producing `Join` and maintained barriers. Do not infer that every Set-returning operator has free full-row occurrence evidence.
2. Revisit `Group` only through maintained-group support/root reuse; do not reintroduce the rejected build-a-new-root prototype.
3. Derive Bag occurrence authority from maintained multiplicity/occurrence state where that state already exists; plain Bag operators must not be forced to build a quotient merely for API symmetry.
4. Wire execution-issued/shared occurrence authority through runtime current-relation ownership so Query/Watch/Change receive one root automatically.
5. Continue workload-weighted compaction estimation and factorized/streaming global `RelExpr` preparation.

### OPEN — DEFERRED / RETURN AFTER CURRENT LINE

- durable physical atoms + durable `RealizationRoot`, crash/root-switch matrix, single-file compaction and encrypted/authenticated atom/root metadata;
- history retention evolution to `HistoricalRevisionRoot -> RealizationRoot -> PhysicalAtoms`, without a second history store;
- Context reference/optional-reference patches, relationship mutation calculus, safe create/delete and final Database/Context DX;
- field-granular certified change coordinates;
- DB-owned granular authorization over query/change/history/watch/hosted-session coordinates;
- deterministic regex/`Matches`, richer serialized Semantic Rules, entity/model invariants and transaction `require`;
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics over the same runtime migration model;
- final Python/.NET/Studio surfaces, backup/restore/corruption UX, public perf/binary-size budgets, native Windows secure-memory expansion.

### SUPERSEDED / REJECTED / DO NOT EXTEND

- REJECTED: direct `Group -> new occurrence root` publication when the executor does not already own that persistent root; measured regression is material.
- SUPERSEDED: realization-side operator-name routing for occurrence certificates. Capability is now owned by prepared exact execution.
- Still rejected/superseded: forced Bag quotient construction, optional write-overlay preparation, maintained-position -> handle translation, second physical Γ authority, fixed-N/depth compaction, source-A inverse/provenance routing, SQL fallback, migration progress journals, Context-as-auth and host callback validation.

### PERFORMANCE BASELINES TO PRESERVE

- P408 Set Union 100k execution certificate: **73.1–78.7 ms** versus **261.5–270.9 ms** old two-pass path.
- P409 cheap-I64 100k/50k-output warm runs: Set projection **~0.97–1.06x**, `Distinct` **~0.94–1.00x** versus evaluate+build; treat these as same-cost-class rather than claimed acceleration.
- REJECTED P409 `Group(count)` certificate prototype: **~1.50–1.54x** baseline after group-key reuse.
- P407 hot current-B write path remains semantic prepare ~5.4 us, root clone ~2.5–2.9 us, install ~11.9–13.2 us.
- P406 depth-1024 mixed-read and compaction ranges remain regression gates.

### LEDGER CARRY RULE

Every successor PASS must carry all unresolved Context/Auth/Semantic Rules/history/bindings/durability/product lines. A line disappears only as `CLOSED`, `SUPERSEDED`, or `REJECTED` with the concrete replacement/reason.

### NEXT RECOMMENDED PASS

PASS410: hostile Set `Difference` and maintained support boundaries. Reuse full-row canonical keys only if the operator already computes them as part of its exact semantics; then inspect whether `AntiJoin` and Set-producing `Join` can compose occurrence evidence from child certificates instead of canonicalizing output. In parallel, investigate maintained `Group` state as the only acceptable route for reviving Group certificate publication without the P409 regression.

## PASS410 — Set Difference occurrence certificate + composition boundary audit

### CLOSED THIS PASS

- Set `Difference` now emits an execution-issued `RelationOccurrenceCertificate` from the exact full-row Γ keys it already computes for support subtraction. Surviving left-row keys are reused directly; target `RelationBaseWitness` therefore does not canonicalize the materialized output a second time.
- The hostile case uses case-insensitive text equivalence: right `ALPHA` blocks left `Alpha`, while surviving `beta` receives an exact generation-zero occurrence handle and the adopted witness shares the certificate persistent root.
- `PreparedRelExpr::emits_occurrence_certificate()` now includes Set `Difference`; realization remains capability-driven and contains no Difference-specific routing.
- `AntiJoin` and Set-producing `Join` were audited and deliberately left without a direct certificate capability. Their current exact algorithms canonicalize blocker/join coordinates, not complete output rows. A direct full-row certificate would merely move the later witness canonicalization into the operator rather than reuse existing proof work.
- Maintained `Group` was re-audited: it owns persistent group-key lookup and group buckets, but not a persistent full result-row occurrence root. This does not overturn P409's rejection of constructing a fresh root solely for certificate symmetry.

### HOSTILE / PERFORMANCE RESULT

100k-row cheap-I64 Set Difference fixture, right side selecting one Γ class, release warm runs:

- old `Difference -> RelationValue -> RelationBaseWitness::build`: late warm runs roughly **117.8–137.2 ms**;
- execution-issued Difference certificate + root adoption: roughly **106.4–117.7 ms**;
- late-run ratio roughly **0.84–0.98x**. Earlier cold/allocator-sensitive samples showed larger gains and are not used as the conclusion.

The accepted conclusion is removal of a duplicate semantic Γ pass with no stable regression and a modest warm-path improvement on this fixture, not a universal multiplicative speedup claim.

### OPEN — IMMEDIATE

1. Derive compositional occurrence evidence for `AntiJoin`: if the left child already owns a full-row occurrence certificate/witness, filter that authority by exact blocker support rather than canonicalizing surviving rows again. Do not build a fresh full-row root inside AntiJoin merely for API uniformity.
2. Derive Set `Join` output occurrence keys from child full-row occurrence evidence plus the exact join fiber/product law. This requires a stable output-handle product/ordering law; join-column canonical keys alone are insufficient.
3. Revisit maintained `Group` only if its maintained result state itself becomes the shared occurrence authority; direct construction of another result root remains rejected.
4. Derive Bag occurrence authority from already-owned multiplicity/occurrence state where available; plain Bag operators must not be forced to compute quotient support they otherwise do not need.
5. Wire shared/execution-issued occurrence authority into the universal runtime current-relation owner so Query/Watch/Change receive one root by construction.
6. Continue factorized/streaming general `RelExpr` preparation and workload-weighted overlay compaction economics.

### OPEN — DEFERRED / RETURN AFTER CURRENT LINE

- durable physical atoms + durable `RealizationRoot`, crash/root-switch matrix, single-file compaction and encrypted/authenticated atom/root metadata;
- history retention evolution to `HistoricalRevisionRoot -> RealizationRoot -> PhysicalAtoms`, without a second history store;
- Context reference/optional-reference patches, relationship mutation calculus, safe create/delete and final Database/Context DX;
- field-granular certified change coordinates;
- DB-owned granular authorization over query/change/history/watch/hosted-session coordinates;
- deterministic regex/`Matches`, richer serialized Semantic Rules, entity/model invariants and transaction `require`;
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics over the same runtime migration model;
- final Python/.NET/Studio surfaces, backup/restore/corruption UX, public perf/binary-size budgets, native Windows secure-memory expansion.

### SUPERSEDED / REJECTED / DO NOT EXTEND

- SUPERSEDED for Set Difference: `evaluate complete survivors -> RelationBaseWitness::build -> canonicalize every survivor again`.
- REJECTED for current AntiJoin/Join executors: constructing a fresh full-output Γ root when execution only owns blocker/join-coordinate canonicalization. The acceptable route is composition from child full-row occurrence evidence.
- REJECTED P409: direct `Group -> fresh occurrence root` publication when maintained/exact Group does not already own that root.
- Still rejected/superseded: forced Bag quotient construction, optional write-overlay preparation, maintained-position -> second handle-witness translation, second physical Γ authority, fixed-N/depth compaction, source-A inverse/provenance routing, SQL fallback, migration progress journals, Context-as-auth and host callback validation.

### PERFORMANCE BASELINES TO PRESERVE

- P408 Set Union 100k: **73.1–78.7 ms** certified vs **261.5–270.9 ms** old two-pass path.
- P409 Set projection / Distinct remain accepted same-cost-class certificate paths.
- P410 Set Difference late warm ratio: **~0.84–0.98x**, with duplicate output canonicalization removed.
- REJECTED P409 Group(count) certificate prototype: **~1.50–1.54x** baseline.
- P407 hot current-B write: semantic prepare ~5.4 us; root clone ~2.5–2.9 us; install ~11.9–13.2 us.
- P406 depth-1024 mixed-read/compaction ranges remain regression gates.

### LEDGER CARRY RULE

Every successor PASS report and master handoff must carry all unresolved Context/Auth/Semantic Rules/history/bindings/durability/product lines. An item disappears only when explicitly marked `CLOSED`, `SUPERSEDED`, or `REJECTED` with the concrete replacement/reason.

### NEXT RECOMMENDED PASS

PASS411: compositional certificate propagation rather than more operator-local canonicalization. Start with `AntiJoin` whose result is a filtered left relation: preserve/filter the left child's full-row occurrence authority when that authority exists. Then derive the Set `Join` handle-product law from child certificates and exact join fibers. If a child lacks reusable full-row evidence, keep one owner-build instead of introducing a hidden second semantic engine.

## PASS411 — compositional AntiJoin certificate + single final occurrence-root materialization

### CLOSED THIS PASS

- Set `AntiJoin` can now emit exact occurrence evidence when its **left child already emits full-row Γ evidence**. AntiJoin reuses the child full-row canonical keys and computes only its own blocker-coordinate law; surviving rows are not full-row canonicalized again.
- Nested AntiJoin propagation is recursive: an AntiJoin whose left subtree is itself certificate-capable remains certificate-capable. A direct `Scan` left child is still explicitly unsupported in the standalone exact executor because that executor has no physical/shared Scan occurrence authority to reuse.
- Hostile performance rejected the first correct implementation, which materialized one persistent occurrence root at the child and another at AntiJoin. The accepted architecture therefore splits certificate propagation into two layers:
  1. transient row-aligned full-row canonical keys compose inside the `RelExpr` tree;
  2. one `RelationOccurrenceCertificate` persistent `class -> StableRowHandle` root is materialized only at the outer adoption boundary.
- `RelationBaseWitness` still receives the same final persistent occurrence root and shares it exactly. Physical handles remain dense generation-zero handles of the **derived output relation**; child handles are semantic evidence, not incorrectly reused as target physical row identity.
- Set Union, Set Difference, `Distinct`, and Set `Project` now use the same transient-composition/final-root law, removing the architectural source of nested-root amplification rather than special-casing AntiJoin.

### HOSTILE / PERFORMANCE RESULT

100k-row compositional AntiJoin fixture (`Distinct(Bag)` left child, case-insensitive text blocker, 10% blocked), release:

- REJECTED first prototype (`child persistent root -> AntiJoin second persistent root`): **1.507x, 2.280x, 2.198x** versus evaluate + final witness build;
- accepted transient proof / one final root: after warm-up **~0.882–0.979x** baseline; cold/allocator-sensitive first samples were much lower and are not used as the conclusion.

Regression controls after the certificate representation change:

- Set Union 100k control run: **~0.258x** baseline, preserving the P408 strong reuse win;
- `Distinct` late warm: **~0.994–1.008x**;
- Set `Project` late warm: **~1.018–1.077x** on cheap `I64Exact` (same cost class; no acceleration claim);
- Set `Difference` late warm: **~0.845–0.942x**.

The accepted conclusion is structural: nested certificate composition no longer forces nested persistent occurrence roots or a second full-row Γ pass.

### OPEN — IMMEDIATE

1. **Set Join compositional certificate**: derive output full-row canonical keys from left/right child full-row canonical keys along exact join fibers. Do not invent an operator-local full-output canonicalization pass. P411 removes the need for a persistent stable-handle product at internal nodes: target dense handles can be minted once at final certificate materialization.
2. **Physical/current Scan certificate seeding**: the standalone exact `FiniteModel` Scan intentionally has no occurrence authority. Runtime/current-relation execution should be able to seed transient canonical-key evidence from the already-owned shared `RelationBaseWitness`/current physical relation root, so ordinary `Join(Scan, Scan)` can participate without rebuilding support.
3. Generalize the transient certificate algebra through nested binary/unary trees and hostile-benchmark depth/composition. One final persistent occurrence root per prepared target remains the invariant.
4. Revisit maintained `Group` only if maintained result state itself can provide row-aligned full-result evidence; P409's fresh-root Group prototype remains rejected.
5. Derive Bag occurrence authority from already-owned multiplicity/occurrence state where available; do not force quotient work into plain Bag operators.
6. Wire the shared certificate/current relation authority through runtime Query/Watch/Change ownership, then return to factorized/streaming global preparation and workload-weighted overlay compaction economics.

### OPEN — DEFERRED / RETURN AFTER CURRENT LINE

- durable physical atoms + durable `RealizationRoot`, crash/root-switch matrix, single-file compaction and encrypted/authenticated atom/root metadata;
- history retention evolution to `HistoricalRevisionRoot -> RealizationRoot -> PhysicalAtoms`, without a second history store;
- Context reference/optional-reference patches, relationship mutation calculus, safe create/delete and final Database/Context DX;
- field-granular certified change coordinates;
- DB-owned granular authorization over query/change/history/watch/hosted-session coordinates;
- deterministic regex/`Matches`, richer serialized Semantic Rules, entity/model invariants and transaction `require`;
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics over the same runtime migration model;
- final Python/.NET/Studio surfaces, backup/restore/corruption UX, public perf/binary-size budgets, native Windows secure-memory expansion.

### SUPERSEDED / REJECTED / DO NOT EXTEND

- REJECTED P411 prototype: building a persistent occurrence root at every certificate-capable internal RelExpr node and then rebuilding/filtering another persistent root in its parent.
- SUPERSEDED certificate representation assumption: persistent `class -> handle` is the **final physical adoption representation**, not the universal internal composition representation. Internal composition uses row-aligned canonical full-row evidence and materializes the persistent root once.
- Still rejected: operator-local full-row canonicalization when only blocker/join-coordinate support exists; direct Group fresh-root publication; forced Bag quotient construction; optional write-overlay preparation; second physical Γ authority; fixed-N/depth compaction; source-A inverse/provenance routing; SQL fallback; migration progress journals; Context-as-auth; host callback validation.

### PERFORMANCE BASELINES TO PRESERVE

- P411 AntiJoin accepted warm ratio: **~0.882–0.979x** evaluate+build baseline; nested-root prototype **1.51–2.28x REJECTED**.
- P408 Set Union strong reuse remains roughly **0.26x** in the P411 control run.
- P409/P410 `Distinct`/Set Project/Set Difference remain same-cost-class or modestly faster while avoiding duplicate full-row Γ passes.
- P407 hot current-B write: semantic prepare ~5.4 us; root clone ~2.5–2.9 us; install ~11.9–13.2 us.
- P406 depth-1024 mixed-read/compaction ranges remain regression gates.

### LEDGER CARRY RULE

Every successor PASS report and master handoff must carry all unresolved Context/Auth/Semantic Rules/history/bindings/durability/product lines. An item disappears only when explicitly marked `CLOSED`, `SUPERSEDED`, or `REJECTED` with the concrete replacement/reason.

### NEXT RECOMMENDED PASS

PASS412: derive **Set Join compositional canonical-key algebra** from child certificates plus exact join fibers. Do not build persistent roots inside child/join nodes; keep one final root materialization. If child evidence is absent, remain explicit/fail-clean rather than recanonicalizing the full Join output under a hidden fast-path label.

### PROVISIONAL PASS FORECAST — NOT A COMMITMENT

- **P412:** Set Join compositional certificate / join-fiber product law.
- **P413:** seed transient certificate evidence from current physical `Scan` authority and hostile nested RelExpr trees.
- **P414:** runtime current-relation integration so Query/Watch/Change/prepared migration share the same occurrence authority automatically.
- **P415+:** depending on hostile results, either maintained Group/Bag evidence reuse or return to factorized/streaming general preparation before durability.

This forecast is deliberately non-authoritative; hostile/R&D findings may reorder or reject items.

## PASS412 — compositional Set Join certificate + contiguous quotient lowering

### CLOSED THIS PASS

- Set `JoinEq` now emits transient full-row Γ evidence when **both child subtrees are certificate-capable Sets**. The output full-row canonical law is compositional rather than re-evaluated:

  ```text
  key(join(left_row, right_row))
      = key(left_row) ++ key(right_row)
  ```

  This is exact because Set Join preserves both complete child rows and its result column equivalences are exactly the left/right equivalence vectors concatenated in the same order.
- Join still computes only the join-coordinate canonical key needed to form exact fibers. Full output canonicalization is not performed inside Join.
- Internal child physical handles are **not** multiplied/reused. P411's law remains: transient canonical evidence composes inside the `RelExpr` tree and fresh dense generation-zero target handles are minted once at final occurrence-certificate materialization.
- `PreparedRelExpr::emits_occurrence_certificate()` now makes Set Join capability structural: both children must independently emit reusable full-row evidence. `Join(Scan, certified-child)` therefore remains explicit/fail-clean until current physical Scan authority can seed evidence.
- Final certificate adoption now recognizes already strictly-sorted unique row-aligned canonical keys and avoids an unnecessary second sort. Unsorted evidence retains the existing exact sort + duplicate rejection path.
- `distinct_rows_with_canonical_keys` was simplified from a node-heavy `BTreeMap<key,row>` quotient to one contiguous `Vec<(key,row)> -> sort -> dedup` lowering. This is the same mathematical quotient and materially reduced the child-certificate payer exposed by Join.

### HOSTILE / PERFORMANCE RESULT

100k one-to-one Set Join fixture with `I64Exact`, both children `Distinct(Bag)`:

- first correct composition before quotient cleanup: warm **~1.086–1.091x** evaluate+final-witness baseline — not accepted as final;
- sorted final-adoption optimization alone improved late warm to about **1.05–1.07x**, still not accepted as sufficient;
- after contiguous `Vec/sort/dedup` child quotient lowering, repeated warm runs: **~0.807–0.878x** baseline.

Cold/allocator-sensitive first samples (`~0.46–0.61x`) are not used as the conclusion.

P409/P410 controls after the quotient change remain healthy on the 100k cheap-I64 fixture:

- `Distinct`: late warm about **0.97–1.00x**;
- Set `Project`: late warm about **0.86–0.97x** in this run;
- Set `Difference`: noisy but no stable regression, late samples approximately **0.73–1.01x**.

The accepted result is therefore both architectural and performance-positive: Set Join removes the second full-output Γ canonicalization and the generic quotient helper is simpler/faster rather than adding an operator-specific fast path.

### OPEN — IMMEDIATE

1. **Current physical Scan certificate seeding.** A standalone exact `FiniteModel` `Scan` still has no occurrence evidence. Seed transient row-aligned canonical evidence from the already-owned current relation `RelationBaseWitness`/physical authority so ordinary `Join(Scan, Scan)`, `AntiJoin(Scan, ...)`, and nested trees compose without rebuilding support.
2. **Nested certificate normalization hostile.** Exercise mixed `Union -> Difference -> AntiJoin -> Join -> Project/Distinct` trees and ensure there is exactly one final persistent occurrence-root materialization, no hidden recanonicalization, and bounded transient proof amplification.
3. **Runtime current-relation integration.** Query/Watch/Change/prepared migration should receive the same current relation occurrence authority automatically rather than the exact standalone executor inventing parallel support.
4. Revisit maintained `Group` only if maintained group result state itself can export already-owned row-aligned full-result evidence; P409's fresh-root Group prototype remains rejected.
5. Derive Bag occurrence evidence only from already-owned multiplicity/occurrence state; do not force quotient construction into ordinary Bag operators.
6. Then return to factorized/streaming general preparation and workload-weighted overlay compaction before durable physical realization authority.

### OPEN — DEFERRED / RETURN AFTER CURRENT LINE

- durable physical atoms + durable `RealizationRoot`, crash/root-switch matrix, single-file compaction and encrypted/authenticated atom/root metadata;
- history retention evolution to `HistoricalRevisionRoot -> RealizationRoot -> PhysicalAtoms`, without a second history store;
- Context reference/optional-reference patches, relationship mutation calculus, safe create/delete and final Database/Context DX;
- field-granular certified change coordinates;
- DB-owned granular authorization over query/change/history/watch/hosted-session coordinates;
- deterministic regex/`Matches`, richer serialized Semantic Rules, entity/model invariants and transaction `require`;
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics over the same runtime migration model;
- final Python/.NET/Studio surfaces, backup/restore/corruption UX, public perf/binary-size budgets, native Windows secure-memory expansion.

### SUPERSEDED / REJECTED / DO NOT EXTEND

- REJECTED P412 intermediate: accept compositional Join merely because it is mathematically correct while cheap-I64 warm performance remains ~1.09x baseline. The generic quotient payer was removed before acceptance.
- SUPERSEDED in certificate-capable Set Join: `evaluate joined rows -> RelationBaseWitness::build -> canonicalize every complete joined row again`.
- Still rejected: persistent root at every internal certificate node; operator-local full-row canonicalization when only join/blocker-coordinate support exists; direct fresh-root Group publication; forced Bag quotient construction; optional write-overlay preparation; second physical Γ authority; fixed-N/depth compaction; source-A inverse/provenance routing; SQL fallback; migration progress journals; Context-as-auth; host callback validation.

### PERFORMANCE BASELINES TO PRESERVE

- P412 Set Join 100k late warm certified/baseline: **~0.807–0.878x** after contiguous quotient lowering.
- P411 AntiJoin accepted warm ratio: **~0.882–0.979x**; nested persistent-root prototype remains **1.51–2.28x REJECTED**.
- P408 Set Union strong reuse remains the major certificate win (~0.26x in later controls).
- P409/P410 Distinct/Set Project/Set Difference remain same-cost-class or faster without duplicate full-row Γ passes.
- P407 hot current-B write: semantic prepare ~5.4 us; root clone ~2.5–2.9 us; install ~11.9–13.2 us.
- P406 depth-1024 mixed-read/compaction ranges remain regression gates.

### LEDGER CARRY RULE

Every successor PASS report and master handoff must carry all unresolved Context/Auth/Semantic Rules/history/bindings/durability/product lines. An item disappears only when explicitly marked `CLOSED`, `SUPERSEDED`, or `REJECTED` with the concrete replacement/reason.

### NEXT RECOMMENDED PASS

PASS413: seed transient full-row canonical evidence from the **already-owned current physical Scan occurrence authority**, rather than teaching standalone `Scan` to recanonicalize rows. Then hostile mixed nested trees so Join/AntiJoin composition can start directly from ordinary current relations and still materialize one persistent occurrence root only at the final target boundary.

### PROVISIONAL PASS FORECAST — NOT A COMMITMENT

- **P413:** current physical `Scan` certificate seeding + `Join(Scan,Scan)` / `AntiJoin(Scan,...)` hostile.
- **P414:** nested certificate-tree normalization, transient-proof memory/depth hostile, and runtime Query/Watch/Change wiring to one current occurrence authority.
- **P415:** depending on hostile results, maintained Group/Bag evidence reuse; otherwise return directly to factorized/streaming global relation preparation and compaction economics.
- **P416+:** once current runtime realization authority is coherent, begin the durable physical-atom/`RealizationRoot` line; if runtime integration exposes semantic-coordinate debt, resolve that first rather than forcing durability.

This forecast is guidance only. Hostile/R&D evidence may reorder, merge, or reject these passes.

## PASS413 — physical Scan occurrence seeding R&D + hot-path rejection

### CLOSED THIS PASS

- Added an exact `RelationScanOccurrenceSeed` calculus in `kernel-query`. A current `RelationBaseWitness` plus the exact logical `StableRowHandle` order can derive row-aligned full-row canonical evidence without invoking Γ canonicalizers again.
- `PreparedRelExpr` now has an explicit seeded certificate capability. `Scan` itself still does **not** synthesize evidence from rows; `Join(Scan, Scan)` / `AntiJoin(Scan, ...)` become certificate-capable only when the caller supplies exact physical Scan seeds.
- Seed validation is exact: relation id, result type, semantic context, live occurrence cardinality, stable-handle identity/generation and one-to-one live handle coverage must match. Missing/forged evidence remains fail-closed.
- A correctness hostile proves seeded direct current-Set scans compose through both Set Join and AntiJoin and produce the same exact result/certificate as ordinary evaluation.
- Hostile performance rejected automatic use of this seed path in `kernel-realization`; the temporary factorized auto-wiring was removed before freeze. P412 production behavior therefore remains unchanged.

### HOSTILE / PERFORMANCE RESULT

100k-row one-to-one `Join(Scan, Scan)` with cheap `I64Exact`, source witness already built:

- first correct reverse translation via `BTreeMap<StableRowHandle, key>`: warm certified/baseline **~1.57–1.67x** — REJECTED;
- density-aware direct slot scatter/gather removed O(N log N) reverse lookup, but full end-to-end remained **~1.32–1.38x** in late warm runs — still REJECTED for automatic production adoption;
- measured late-run decomposition: source seed derivation **~32–34 ms**, certified Join execution **~127–129 ms**, final witness adoption ~**0.02 ms**, versus baseline evaluate+final witness **~119–123 ms**.

The obstruction is now precise: physical authority already owns the Γ quotient, but converting `class -> handles` back into a fresh row-aligned key vector before every exact execution is itself an O(data) copy. Eliminating canonicalization is insufficient if evidence is then rematerialized per query.

### OPEN — IMMEDIATE

1. **Structural row-aligned evidence ownership.** Current-relation authority must retain/share an execution-friendly row-aligned canonical evidence representation, or an equivalent borrowed/persistent view, so Scan seeding is O(1)/structurally shared rather than O(data) reconstruction. Do not add an independently mutable second Γ authority.
2. Re-run `Join(Scan,Scan)` / `AntiJoin(Scan,...)` hostile only after that representation exists; automatic realization/runtime wiring remains forbidden until cheap-I64 is at least native cost class.
3. Then hostile mixed nested trees (`Union -> Difference -> AntiJoin -> Join -> Project/Distinct`) for proof depth/memory amplification and exactly one final persistent occurrence-root materialization.
4. Runtime Query/Watch/Change integration follows only after Scan evidence sharing is performance-clean.
5. Maintained Group/Bag evidence reuse remains conditional on already-owned result/multiplicity authority; do not force fresh quotient work.
6. Then return to factorized/streaming global preparation and workload-weighted overlay compaction before durable physical realization authority.

### OPEN — DEFERRED / RETURN AFTER CURRENT LINE

- durable physical atoms + durable `RealizationRoot`, crash/root-switch matrix, single-file compaction and encrypted/authenticated atom/root metadata;
- history retention evolution to `HistoricalRevisionRoot -> RealizationRoot -> PhysicalAtoms`, without a second history store;
- Context reference/optional-reference patches, relationship mutation calculus, safe create/delete and final Database/Context DX;
- field-granular certified change coordinates;
- DB-owned granular authorization over query/change/history/watch/hosted-session coordinates;
- deterministic regex/`Matches`, richer serialized Semantic Rules, entity/model invariants and transaction `require`;
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics over the same runtime migration model;
- final Python/.NET/Studio surfaces, backup/restore/corruption UX, public perf/binary-size budgets, native Windows secure-memory expansion.

### SUPERSEDED / REJECTED / DO NOT EXTEND

- REJECTED P413 prototype: per-execution `BTreeMap<StableRowHandle, CanonicalRowKey>` reverse translation from witness to Scan evidence.
- REJECTED P413 production wiring: even the O(N) dense-slot translation is too expensive to enable automatically on the hot exact path (~1.32–1.38x late warm baseline).
- Do not call “no recanonicalization” a performance closure while row-aligned evidence is rebuilt O(data) per execution.
- Still rejected: persistent root at every internal certificate node; operator-local full-row canonicalization when reusable evidence exists; fresh-root Group publication; forced Bag quotient construction; optional write-overlay preparation; independent second physical Γ authority; fixed-N/depth compaction; source-A inverse/provenance routing; SQL fallback; migration progress journals; Context-as-auth; host callback validation.

### PERFORMANCE BASELINES TO PRESERVE

- P413 rejected Scan-seed end-to-end late warm: **~1.32–1.38x** baseline after dense-slot optimization; source seed construction alone **~32–34 ms / 100k**.
- P412 Set Join certified/baseline: **~0.807–0.878x** when child evidence is already available without Scan reconstruction.
- P411 AntiJoin accepted warm ratio: **~0.882–0.979x**.
- P408 Set Union strong reuse remains roughly **~0.26x** in later controls.
- P407 current-B write: semantic prepare ~5.4 us; root clone ~2.5–2.9 us; physical overlay install ~11.9–13.2 us.
- P406 depth-1024 mixed-read/compaction ranges remain regression gates.

### LEDGER CARRY RULE

Every successor PASS report and master handoff must carry all unresolved Context/Auth/Semantic Rules/history/bindings/durability/product lines. An item disappears only when explicitly marked `CLOSED`, `SUPERSEDED`, or `REJECTED` with the concrete replacement/reason.

### NEXT RECOMMENDED PASS

PASS414 should **not** wire the current P413 seed into runtime. First R&D an execution-facing current-relation evidence carrier that shares row-aligned canonical evidence structurally with the same `RelationBaseWitness`/physical authority and advances it with bounded relation deltas. Then repeat the P413 Scan/Join hostile. Only if that removes the ~32–34 ms reconstruction payer should runtime integration proceed.

### PROVISIONAL PASS FORECAST — NOT A COMMITMENT

- **P414:** shared/borrowed row-aligned Scan evidence representation; bounded update law; repeat Scan/Join/AntiJoin perf.
- **P415:** if P414 closes performance, nested certificate-tree hostile + runtime Query/Watch/Change wiring; otherwise continue evidence representation R&D rather than routing around it.
- **P416:** maintained Group/Bag evidence reuse if already-owned support permits it; otherwise factorized/streaming global preparation + overlay cost economics.
- **P417+:** durable physical atoms/`RealizationRoot` only after runtime current-relation authority is coherent and no semantic/write/read routing seam remains.

This forecast is guidance only and may change under hostile evidence.

## PASS414 — structurally shared physical Scan row evidence

### CLOSED THIS PASS

- Current factorized relation authority now retains a `RelationScanOccurrenceSeed` beside its exact `RelationBaseWitness` and bounded relation overlay. The seed's row-aligned canonical evidence is a persistent vector; handing it to exact execution is an O(1) structural clone rather than an O(data) reconstruction.
- Bounded current-B relation writes advance physical logical-row order and row evidence by the same exact prepared transition: each removal uses the physical overlay's actual `swap_remove` position and verifies the prepared Γ key; inserts append the prepared Γ keys. There is no independently mutable second semantic authority.
- General relation preparation constructs the Scan evidence once at physical construction/cutover. General-query composition carries the same evidence into the current target root. Compaction rebuilds witness + row evidence once with the new native base.
- Exact certificate execution now carries transient canonical evidence as `PersistentVec<CanonicalRowKey>`, so seeded `Scan` clones the shared evidence root instead of allocating/cloning 100k keys before every execution.
- Join lowering was adjusted to index the right join fiber and borrow child full-row canonical evidence instead of cloning the whole right evidence into the join bucket.
- Set overlay hostile proves row evidence remains exact after case-insensitive Γ remove/insert and can immediately seed `Join(Scan, Scan)` with exact parity.

### HOSTILE / PERFORMANCE RESULT

100k one-to-one Set `Join(Scan,Scan)`, cheap `I64Exact`:

- P413 rebuilt seed: seed construction remains ~31.6–34.0 ms and late end-to-end ~1.25–1.30x baseline — still rejected.
- P414 shared seed clone: ~0.10 ms.
- after removing whole-right-evidence cloning from Join, late warm shared end-to-end samples: **1.095x / 1.097x** baseline in the final repeated run; this is back inside the previously accepted native-cost class (~<=1.10x).
- earlier allocator/cache warm-up samples were substantially noisier (up to ~1.4x) and are not treated as steady-state conclusions.

The key closure is not that canonical evidence disappeared: it is now constructed once with current physical authority and persistently shared/updated, rather than rebuilt O(data) per exact execution.

### OPEN — IMMEDIATE

1. Mixed nested certificate-tree hostile from ordinary current physical Scans (`Union -> Difference -> AntiJoin -> Join -> Project/Distinct`): prove bounded transient proof amplification and one final persistent occurrence-root materialization.
2. Runtime Query/Watch/Change integration: route the already-owned current relation Scan evidence into exact/current execution automatically without creating a second current-relation authority.
3. Audit maintained Scan/Watch storage binding so the same shared row evidence can be reused rather than separately maintained as positional quotient state where structurally equivalent.
4. Revisit maintained `Group` only if its already-owned result state can export row-aligned full-result evidence without a fresh persistent root; P409 fresh-root Group remains rejected.
5. Bag evidence remains conditional on already-owned multiplicity/occurrence authority; do not force quotient construction into Bag operators.
6. Then return to factorized/streaming global preparation + workload-weighted overlay compaction before durable physical authority.

### OPEN — DEFERRED / RETURN AFTER CURRENT LINE

- durable physical atoms + durable `RealizationRoot`, crash/root-switch matrix, single-file compaction and encrypted/authenticated atom/root metadata;
- history retention as `HistoricalRevisionRoot -> RealizationRoot -> PhysicalAtoms`, no second history store;
- Context reference/optional-reference patches, relationship mutation calculus, safe create/delete and final Database/Context DX;
- field-granular certified change coordinates;
- DB-owned granular authorization over query/change/history/watch/hosted session coordinates;
- deterministic regex/`Matches`, richer serialized Semantic Rules, entity/model invariants and transaction `require`;
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics;
- final Python/.NET/Studio surfaces, backup/restore/corruption UX, public perf/binary-size budgets, native Windows secure-memory expansion.

### SUPERSEDED / REJECTED / DO NOT EXTEND

- SUPERSEDED P413 per-execution witness->row-evidence translation (both BTreeMap and dense slot-table forms) by persistent construction-time/current-owner row evidence.
- REJECTED: independently mutable row-evidence cache beside Γ witness. P414 evidence changes only through the same prepared physical transition and is carried with the current relation owner.
- Still rejected: persistent root at every internal certificate node; operator-local full-row recanonicalization where reusable evidence exists; fresh-root Group publication; forced Bag quotient construction; optional write-overlay preparation; independent second physical Γ authority; fixed-N/depth compaction; source-A inverse/provenance routing; SQL fallback; migration progress journals; Context-as-auth; host callback validation.

### PERFORMANCE BASELINES TO PRESERVE

- P414 shared Scan seed clone: ~0.10 ms / 100k; final late warm `Join(Scan,Scan)` ~1.095–1.097x baseline.
- P413 rejected seed rebuild: ~32–34 ms / 100k and ~1.25–1.30x+ late end-to-end.
- P412 Set Join with already-available child evidence: ~0.807–0.878x.
- P411 AntiJoin: ~0.882–0.979x.
- P408 Set Union strong reuse: ~0.26x in later controls.
- P407 hot current-B write: prepare ~5.4 us; root clone ~2.5–2.9 us; overlay install ~11.9–13.2 us.
- P406 depth-1024 mixed-read/compaction ranges remain gates.

### LEDGER CARRY RULE

Every successor PASS report and master handoff must carry all unresolved Context/Auth/Semantic Rules/history/bindings/durability/product lines. An item disappears only when explicitly marked `CLOSED`, `SUPERSEDED`, or `REJECTED` with the concrete replacement/reason.

### NEXT RECOMMENDED PASS

PASS415: hostile mixed nested certificate trees beginning from **ordinary current physical Scan evidence**, then wire Query/Watch/Change to the same current relation authority only if the nested-tree proof/memory profile stays bounded and there is still one final persistent output-root materialization.

### PROVISIONAL PASS FORECAST — NOT A COMMITMENT

- **P415:** mixed nested certificate-tree hostile + current runtime Query/Watch/Change integration if the hostile remains clean.
- **P416:** maintained Group/Bag evidence reuse if an already-owned authority exists; otherwise factorized/streaming global preparation + overlay/compaction economics.
- **P417:** close remaining runtime/realization ownership seams and history-root interaction found by integration hostile.
- **P418+:** durable physical atoms / durable `RealizationRoot` only after current runtime authority is coherent.

Hostile/R&D evidence may reorder, merge, or reject these forecasts.

## PASS415 — nested certificate-tree normalization + physical owner handoff

### CLOSED THIS PASS

- Nested Set certificate evaluation now reuses child full-row canonical evidence through `Union`, `Difference`, Set `Project`, and redundant Set `Distinct`; those operators no longer obligatorily recanonicalize every intermediate row when exact child proof is already available.
- `Union` performs quotient/dedup directly in canonical key-space; `Difference` subtracts child canonical classes; Set `Project` projects the corresponding canonical-key components by column ordinal before quotienting; Set `Distinct` reuses an already-Set child certificate unchanged when equivalences match.
- Exact `Join`/`AntiJoin` composition from P411/P412 and shared physical Scan evidence from P414 therefore survive a deep mixed tree without persistent-root materialization at internal nodes.
- `prepare_general_relation_factorized` now collects already-owned `RelationScanOccurrenceSeed` values from source physical relation rules and automatically uses seeded certificate execution when `PreparedRelExpr` proves the tree supports it. This is an owner-level handoff, not a second runtime/query engine and not schema-A routing.
- New correctness hostile covers `Scan -> Join -> Project -> Union -> AntiJoin -> Difference -> Project -> Distinct` and proves ordinary evaluation equals seeded certified execution and final witness adoption.

### HOSTILE / PERFORMANCE RESULT

100k cheap-`I64Exact` mixed nested Set tree, final result adopted into one `RelationBaseWitness`:

- release series 1 certified/baseline: **0.896x, 0.855x, 0.846x, 0.834x**;
- release series 2 certified/baseline: **0.862x, 0.858x, 0.850x, 0.832x**.

The accepted conclusion is that certificate depth no longer amplifies repeated full-row Γ canonicalization for the audited operator tree. Transient canonical evidence remains per-result-row data, but one persistent occurrence root is still materialized only at the final physical adoption boundary.

### OPEN — IMMEDIATE

1. **Runtime Query/Watch/Change authority wiring.** Live/runtime execution must obtain the same current physical Scan evidence owner without introducing a parallel query-specific Γ authority or a physical-epoch router.
2. Hostile maintained/watch transitions through nested certificate-capable trees so shared evidence remains exact under source deltas and watch maintenance, not only exact snapshot execution.
3. Audit whether maintained Scan storage support and factorized `RelationBaseWitness + RelationScanOccurrenceSeed` can share one structural owner rather than retaining equivalent persistent indexes in two subsystems.
4. Maintained `Group`/Bag evidence reuse only where existing maintained state already owns the required result/multiplicity proof; P409 fresh Group occurrence-root construction remains rejected.
5. Then resume factorized/streaming global preparation and workload-weighted overlay compaction before durable physical authority.

### OPEN — DEFERRED / RETURN AFTER CURRENT LINE

- durable PhysicalAtoms/RealizationRoot, crash/root-switch matrix, encrypted/authenticated single-file root/atom metadata and compaction;
- historical `HistoricalRevisionRoot -> RealizationRoot -> PhysicalAtoms` retention without a second history store;
- Context reference/optional-reference patches, relationship mutation calculus, field-granular certified changes, safe create/delete and final Database/Context DX;
- DB-owned granular authorization over query/change/history/watch/hosted-session coordinates;
- deterministic regex/`Matches`, richer serialized Semantic Rules, entity/model invariants and transaction `require`;
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics;
- Python/.NET/Studio surfaces, backup/restore/corruption UX, public perf/binary-size budgets, native Windows secure-memory expansion.

### SUPERSEDED / REJECTED / DO NOT EXTEND

- SUPERSEDED in seeded nested Set trees: recanonicalizing rows independently inside `Union`, `Difference`, Set `Project`, or redundant Set `Distinct` when exact child full-row canonical evidence already exists.
- Still rejected: persistent occurrence root at every internal certificate node; per-query Scan evidence reconstruction; fresh Group occurrence root; forced Bag quotient construction; independent second physical Γ authority; fixed-N/depth compaction; inverse-A/provenance write routing; SQL fallback; migration progress journal; Context-as-auth; host callback validation.

### PERFORMANCE BASELINES TO PRESERVE

- P415 mixed nested tree certified/baseline: **~0.832–0.896x** across two 100k release series.
- P414 shared `Join(Scan,Scan)` late warm: **~1.095–1.097x**; Scan evidence clone ~0.10 ms instead of P413 ~32–34 ms rebuild.
- P412 Set Join: **~0.807–0.878x** when child evidence is already available.
- P411 AntiJoin: **~0.882–0.979x**; nested persistent-root prototype **1.51–2.28x REJECTED**.
- P407 current-B write: semantic prepare ~5.4 us; root clone ~2.5–2.9 us; overlay install ~11.9–13.2 us.
- P406 depth-1024 mixed-read/compaction economics remain regression gates.

### LEDGER CARRY RULE

Every successor PASS report/master handoff must carry all unresolved Context/Auth/Semantic Rules/history/bindings/durability/product lines. A line disappears only when explicitly `CLOSED`, `SUPERSEDED`, or `REJECTED` with its concrete replacement/reason.

### NEXT RECOMMENDED PASS

PASS416 should wire runtime exact Query/Watch/Change to the same current physical Scan evidence authority and hostile delta/watch maintenance through a nested certificate-capable tree. Do not create a runtime-side occurrence cache or route by migration epoch.

### PROVISIONAL PASS FORECAST — NOT A COMMITMENT

- **P416:** runtime Query/Watch/Change authority wiring + nested maintained/watch hostile.
- **P417:** maintained Group/Bag evidence reuse if already-owned state permits; otherwise return to factorized/streaming general preparation + overlay/compaction R&D.
- **P418:** close runtime/history-root ownership seams exposed by integration hostile.
- **P419+:** durable PhysicalAtoms / durable `RealizationRoot` only once current runtime authority is coherent.

Hostile/R&D evidence may reorder or reject this forecast.

## PASS416 — runtime current-relation Scan evidence authority

### CLOSED THIS PASS

- `kernel-plan::RuntimeRevisionBundle` now publishes row-aligned `RelationScanOccurrenceSeed` evidence atomically beside the logical revision, `PhysicalStore`, `relation_bases`, and maintained materializations. There is no runtime-global mutable Γ cache and no migration-epoch router.
- Bootstrap constructs each runtime Scan seed once from the already-built exact `RelationBaseWitness`. Live exact Query collects only the dependency seeds required by its prepared expression and uses seeded canonical-evidence execution when `PreparedRelExpr::emits_occurrence_certificate_with_scan_seeds` proves the capability; unsupported algebra keeps the existing explicit exact evaluator rather than attempting a hidden certificate path.
- Relation-only and mixed runtime transitions advance the same bundle-owned Scan evidence from prior canonical keys plus the exact `RelationDelta`: unchanged rows are not Γ-canonicalized again, removals follow logical survivor order, and inserted rows alone cross Γ before the new immutable bundle is published.
- Runtime materialization/Watch handle binding remains independent unless its physical handle domain is proven compatible. A prototype that attached the current relation base witness was hostile-rejected because late materialization registration and historical Watch replay can observe reused physical generations that are not the semantic witness handle domain.
- `PreparedRelExpr::evaluate_seeded` exposes value-only seeded execution so ordinary live reads do not materialize a final persistent occurrence certificate merely to return query rows.
- Hostile correctness added for survivor-order Scan-seed advancement under case-insensitive Γ. Full kernel-query, kernel-plan, cfmd-runtime end-to-end, and workspace all-target checks pass on the frozen code.

### HOSTILE / PERFORMANCE RESULT

- 100k-row one-remove/one-insert runtime Scan-seed publication projection, release: first run 9.772 ms; warm runs **4.798 / 4.956 / 5.018 / 4.903 ms**.
- This is materially below the rejected P413 per-query reconstruction (~32–34 ms), and it is paid once per publication rather than once per exact execution.
- It is nevertheless still **O(N) in live rows** because runtime logical relation order is survivor-order + append, not factorized-overlay swap-remove order. P416 therefore closes authority coherence, not the final bounded-update representation problem.

### REJECTED / SUPERSEDED

- **REJECTED:** binding product `QueryWatch` Scan leaves directly to current physical handles/base witness. Durable watch catch-up replays semantic causal-history deltas, which intentionally do not contain storage-resolved handle receipts. The prototype caused three exact watch regressions (`custom_publication_notifier...`, `watch_recv_blocks...`, `watch_lag...`) and was fully reverted. Watch initialization still executes through the seeded live Query path; maintained historical evolution remains semantic until a dedicated persistent semantic-seed handoff exists.
- **SUPERSEDED:** P413 per-execution witness -> row-evidence reconstruction as a runtime strategy. P416 stores the current evidence in the immutable runtime root and structurally clones it for reads.
- Still rejected: second independently mutable runtime Γ cache; nested persistent occurrence roots; source-schema current routing; SQL writable-view/inverse migration fallback.

### OPEN — IMMEDIATE

1. Replace the O(N) runtime survivor projection with an order-statistic/persistent-sequence representation (or a stronger equivalent) so current relation row evidence advances in O(delta log N) / bounded structural work while preserving logical Scan order. Benchmark write-heavy 100k/1M cases before acceptance.
2. Give `QueryWatch` a **semantic** persistent Scan-evidence seed at anchor/build time, structurally shared from the runtime seed but advanced by durable semantic deltas; do not attach current physical handles to historical replay state. Prove exact/watch parity over lagging multi-transition catch-up.
3. Audit maintained Scan `CanonicalRowPositionIndex`: if its by-position canonical evidence can structurally share the same semantic seed without duplicating an independently mutable authority, do so; otherwise record the irreducible maintained-state reason.
4. Revisit maintained `Group`/Bag evidence only when existing maintained support can export exact result evidence without a fresh full persistent root. P409 fresh-root Group remains rejected.
5. After runtime Scan evidence + Watch anchor semantics are bounded, return to factorized/streaming global preparation and workload-weighted overlay compaction economics before durable physical authority.

### OPEN — DEFERRED / MANDATORY CARRY

- durable physical atoms + durable `RealizationRoot`, crash/root-switch matrix, single-file compaction, encrypted/authenticated atom/root metadata;
- history retention as `HistoricalRevisionRoot -> RealizationRoot -> PhysicalAtoms`, with no second history store;
- Context reference/optional-reference patches, relationship mutation calculus, safe create/delete and final Database/Context DX;
- field-granular certified change coordinates;
- DB-owned granular authorization over query/change/history/watch/hosted-session coordinates;
- deterministic regex/`Matches`, richer serialized Semantic Rules, entity/model invariants and transaction `require`;
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics;
- final Python/.NET/Studio surfaces, backup/restore/corruption UX, public perf/binary-size budgets, native Windows secure-memory expansion.

### PROVISIONAL PASS FORECAST — NOT A COMMITMENT

- **P417:** bounded/order-statistic runtime Scan evidence + semantic Watch seed handoff; hostile publication/read/watch benchmarks.
- **P418:** maintained Scan/Group/Bag evidence ownership cleanup if P417 exposes reusable support; otherwise factorized/streaming global preparation + overlay/compaction debt.
- **P419:** close remaining runtime/history-root ownership seams and mixed long-lived watch/query/change hostile.
- **P420+:** durable PhysicalAtoms / durable `RealizationRoot` only after current runtime evidence authority is bounded and coherent.

The forecast is guidance only; hostile evidence may change the order.

## PASS417 — witness-owned logical Scan evidence + semantic Watch seed

### CLOSED THIS PASS

- `RelationBaseWitness` now owns two persistent projections of one exact Γ occurrence authority: canonical class -> stable occurrences and stable occurrence -> canonical row key. The second projection is updated in the same `apply_delta_supports` transition and is not independently mutable.
- For Set relations, monotone semantic occurrence slots define the exact logical survivor-order + append order. `logical_scan_occurrence_seed()` therefore returns an O(1) persistent view instead of rebuilding row evidence.
- Runtime Change publication now advances `RelationBaseWitness` once and derives the next Scan seed directly from the advanced witness. The P416 O(N) `RelationScanOccurrenceSeed::advance_logical_delta` publication path is no longer used by runtime Set publication.
- Scan evidence now has a stable-order persistent representation in addition to the P414 dense physical representation. Exact execution consumes both through one canonical-evidence interface.
- `MaterializedRelPlanState::build_with_scan_seeds` bootstraps maintained Scan canonical lookup from semantic Scan evidence without physical handles or a physical base-witness binding.
- Product `QueryWatch` uses that semantic seeded bootstrap when all source Set relations expose seeds. Durable watch catch-up remains ordinary semantic delta replay, preserving the P416 historical-authority boundary.
- 100k Set one-row publication benchmark: old dense logical survivor projection ~5.9–7.6 ms warm/cold range in this run; witness-owned advance + seed view first ~0.92 ms, then ~0.005–0.009 ms warm.

### OPEN IMMEDIATE

1. Audit Bag logical occurrence ordering separately. Set monotone-slot order is exact because Set has one occurrence per Γ class; do not generalize the same proof to Bag duplicate occurrence semantics without an explicit multiplicity/order authority.
2. Audit maintained `CanonicalRowPositionIndex` for deeper structural sharing with semantic Scan evidence. P417 avoids recanonicalization but still builds its own maintained position index at Watch bootstrap.
3. Revisit maintained Group only if existing group state can export result-row evidence without a fresh persistent root; P409 fresh-root Group certificate remains rejected.
4. Run long-lived mixed Query/Watch/Change hostile with repeated Set updates, snapshot retention and nested certificate-capable trees; measure evidence memory/root sharing over churn.
5. Return to factorized/streaming global preparation and workload-weighted overlay compaction once the maintained/runtime evidence ownership audit is complete.

### SUPERSEDED / REJECTED

- **SUPERSEDED:** P416 runtime Set seed publication via O(N) logical survivor projection. The runtime now advances the witness and takes an O(1) seed view.
- **REJECTED remains:** current physical handle/base-witness binding into historical Watch replay.
- **REJECTED remains:** treating semantic witness handles as the physical handle domain for late materialization registration.

### PROVISIONAL FORECAST — NOT A COMMITMENT

- **P418:** maintained Group/Bag evidence ownership audit + long-lived mixed Query/Watch/Change hostile; if Bag needs a different occurrence-order algebra, formulate it explicitly instead of forcing the Set law.
- **P419:** close remaining runtime/history-root ownership seams and factorized/streaming global-preparation debt.
- **P420+:** durable PhysicalAtoms / durable `RealizationRoot` only after current runtime + maintained evidence authority remains coherent under long-lived churn.


## PASS418 — Bag FIFO occurrence authority + maintained Group churn hostile

### CLOSED THIS PASS

- `RelationBaseWitness` occurrence buckets are now persistent FIFO queues rather than LIFO `PersistentVec::pop()` buckets. A per-bucket head offset makes oldest-live removal O(1) amortized; geometric tail compaction bounds dead prefixes without turning each removal into O(class multiplicity).
- Bag duplicate Γ-class removal now matches the existing logical relation law exactly: logical delta application removes the first live semantic occurrence in survivor order, and the witness now returns the same oldest stable occurrence. Set behavior is unchanged because a Set class has at most one live occurrence.
- `RelationBaseWitness::logical_scan_occurrence_seed()` is therefore valid for both Set and Bag. Runtime bundle bootstrap/publication now emits Scan evidence for Bag relations from the same witness authority; no Bag-specific cache or second Γ index was introduced.
- Hostile interleaving test `[Alpha, Beta, ALPHA, Gamma] - alpha + Delta` proves Bag witness evidence follows logical `[Beta, ALPHA, Gamma, Delta]`, not stable-slot LIFO order.
- A 128-remove hostile crosses FIFO head compaction and proves row-aligned evidence remains exact after geometric bucket compaction.
- Long-lived Bag -> maintained Group hostile performs 128 remove+append transitions while advancing the witness and maintained group state. Maintained Group output remains semantically equal to exact recompute on every transition, and final Bag Scan evidence exactly matches actual logical row order.
- `kernel-plan` runtime and `cfmd-runtime` Watch/end-to-end regressions pass with Bag seeds enabled.

### HOSTILE / PERFORMANCE RESULT

100k Bag rows, 1024 repeated Γ classes, one-row remove+append:

- legacy dense `RelationScanOccurrenceSeed::advance_logical_delta`: **6.274 / 4.966 / 5.065 / 5.113 / 5.155 ms**;
- witness-owned FIFO advance + logical seed view: **0.886 ms first**, then **0.007 / 0.006 / 0.005 / 0.005 ms** warm.

The accepted conclusion is the same structural one as P417 Set publication: runtime evidence publication is now proportional to touched Γ support plus persistent-path copying instead of a full live-row survivor projection.

### GROUP AUDIT RESULT

- Maintained Group already owns canonical **group-key** lookup plus aggregate state, but it does not own a full canonical output-row proof: the aggregate result component is separate.
- P409 already measured a fresh Group output occurrence root at ~1.50–1.54x baseline. P418 therefore does **not** resurrect a Group certificate merely for API symmetry.
- The 128-transition Bag->Group churn proves Group maintenance composes correctly with seeded Bag Scan evidence without requiring a second output root.

### OPEN IMMEDIATE

1. Audit maintained `CanonicalRowPositionIndex` for structural sharing with witness-owned Scan evidence; bootstrap still materializes a maintained position index once.
2. If Group result evidence is needed by exact/runtime parents, derive it only from already-maintained bucket/aggregate authority with bounded per-changed-group maintenance; do not build a fresh whole-result root.
3. Run retained-reader/snapshot memory hostile over long mixed Query/Watch/Change churn and nested certificate trees; measure persistent root retention and reclamation, not only correctness.
4. Close remaining runtime/history-root ownership seams and resume factorized/streaming global preparation + workload-weighted overlay compaction.
5. Only then reopen durable PhysicalAtoms / durable `RealizationRoot` integration.

### SUPERSEDED / REJECTED

- **SUPERSEDED:** Set-only proof for witness-owned logical Scan evidence. P418 replaces the underlying occurrence selection law with FIFO oldest-live semantics, making the same authority valid for Bag as well.
- **SUPERSEDED:** Bag runtime O(N) seed survivor projection as a required publication path.
- **REJECTED remains:** fresh Group persistent occurrence root; independently mutable Bag multiplicity/order cache; current physical handle binding into historical Watch replay.

### PERFORMANCE BASELINES TO PRESERVE

- P418 Bag witness-owned publication warm: **~0.005–0.007 ms / 100k**, legacy dense projection **~4.97–6.27 ms**.
- P417 Set witness-owned publication warm: **~0.005–0.009 ms / 100k**.
- P415 nested certificate tree: **~0.832–0.896x** evaluate+final-witness baseline.
- P414 shared Scan self-Join late warm: **~1.095–1.097x** baseline.
- P412 Set Join: **~0.807–0.878x**; P411 AntiJoin **~0.882–0.979x**.
- P407 hot current-B write: semantic prepare ~5.4 us; root clone ~2.5–2.9 us; overlay install ~11.9–13.2 us.

### OPEN DEFERRED — MANDATORY CARRY

- durable PhysicalAtoms + durable `RealizationRoot`, crash/root-switch matrix, encrypted/authenticated single-file atom/root metadata and compaction;
- historical `HistoricalRevisionRoot -> RealizationRoot -> PhysicalAtoms` retention without a second history store;
- Context reference/optional-reference patches, relationship mutation calculus, safe create/delete and final Database/Context DX;
- field-granular certified change coordinates;
- DB-owned granular authorization over query/change/history/watch/hosted-session coordinates;
- deterministic regex/`Matches`, richer serialized Semantic Rules, entity/model invariants and transaction `require`;
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics;
- Python/.NET/Studio surfaces, backup/restore/corruption UX, public perf/binary-size budgets, native Windows secure-memory expansion.

### PROVISIONAL FORECAST — NOT A COMMITMENT

- **P419:** retained-snapshot/root-sharing hostile + remaining runtime/history-root ownership seams; factorized/streaming global-preparation debt.
- **P420:** workload-weighted realization compaction/global preparation closure if P419 is clean.
- **P421+:** durable PhysicalAtoms / durable `RealizationRoot` integration only after current/runtime/history authority remains coherent under retained-reader churn.

Hostile/R&D evidence may reorder, merge, or reject this forecast.


## PASS419 — retained root authority + witness-derived runtime Scan evidence

### CLOSED THIS PASS

- Removed `RuntimeRevisionBundle::relation_scan_seeds` as a separately synchronized persistent directory. Runtime Scan evidence now has exactly one semantic owner: `RelationBaseWitness::ordered_occurrences`; Query/Watch derive an O(1) persistent seed view directly from the witness retained by their immutable runtime snapshot.
- `RuntimeRevisionBundle::relation_scan_occurrence_seed` now derives from the retained witness and returns errors fail-closed. There is no query/runtime fallback from a broken witness to row recanonicalization.
- Retained-reader hostile now proves an old `RuntimeRevisionSnapshot` keeps its exact pre-publication Bag witness/evidence after a later atomic commit while the new snapshot sees the advanced witness. Each derived seed structurally shares the logical-order persistent root of its own witness.
- Runtime materialization bootstrap and late materialization registration now use `MaterializedRelPlanState::build_with_scan_seeds` from witness-owned semantic evidence before storage rows are attached. This removes an independent Scan canonicalization pass without binding maintained history to the witness's physical handle domain.
- The superseded standalone `RelationScanOccurrenceSeed::advance_logical_delta` publication path is no longer a production surface; it is test-only reference/benchmark code. Production Change advances the witness authority and derives the next Scan view from it.
- Workspace/all-target compilation and kernel-query/kernel-plan/cfmd-runtime/kernel-realization regression are clean on frozen code.

### HOSTILE / OWNERSHIP RESULT

The P417/P418 representation already made Scan evidence persistent, but P419 found that the runtime still mirrored those views in `relation_scan_seeds`. Although the leaf data shared persistent roots, the directory itself was a second owner that every publication/physical-only root clone had to keep synchronized. That directory is deleted.

Selected runtime law after P419:

```text
RuntimeRevisionSnapshot
    -> RelationBaseWitness
        -> Γ class -> FIFO stable occurrences
        -> stable occurrence -> canonical row key
        -> O(1) RelationScanOccurrenceSeed view

Query / Watch bootstrap / maintained materialization bootstrap
    consume the view
    do not own a second Γ directory.
```

Retained snapshots continue to own their old `Arc<RuntimeRevisionBundle>` and therefore their old witness roots. No current publication mutates or overwrites historical reader evidence.

### OPEN — IMMEDIATE

1. Retained-root **memory** hostile under many long-lived snapshots/watch anchors: quantify live persistent-node retention/reclamation rather than only semantic immutability.
2. Resume factorized/streaming general `RelExpr` preparation debt and close workload-weighted overlay compaction economics; P419 intentionally did not mix this physical work into runtime/history authority cleanup.
3. Maintained `CanonicalRowPositionIndex` remains a maintained-query position structure distinct from witness semantic occurrence ownership. Audit whether construction/update can consume witness canonical evidence without duplicating canonicalization while preserving its position-specific role.
4. Group full result-row evidence remains unavailable unless it can be incrementally derived from already-maintained group + aggregate authority; P409 fresh Group root remains rejected.
5. Only after these gates: durable PhysicalAtoms / durable `RealizationRoot` and historical physical-root retention.

### OPEN — DEFERRED / MANDATORY CARRY

- durable PhysicalAtoms + durable `RealizationRoot`; crash/root-switch matrix; single-file compaction; encrypted/authenticated atom/root metadata;
- historical `HistoricalRevisionRoot -> RealizationRoot -> PhysicalAtoms` retention, without a second history store;
- Context reference/optional-reference patches, relationship mutations, safe create/delete and final Database/Context DX;
- field-granular certified change coordinates;
- DB-owned granular authorization for query/change/history/watch/hosted-session coordinates;
- deterministic regex/`Matches`, richer serialized Semantic Rules, entity/model invariants, transaction `require`;
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics;
- final Python/.NET/Studio surfaces, backup/restore/corruption UX, public perf/binary budgets, native Windows secure-memory expansion.

### SUPERSEDED / REJECTED — DO NOT REINTRODUCE

- SUPERSEDED: bundle-owned `relation_scan_seeds` directory synchronized beside `relation_bases`.
- SUPERSEDED as production publication API: independently advancing `RelationScanOccurrenceSeed` through logical deltas; witness advance is the authority.
- REJECTED: current physical handle/base-witness binding into historical Watch replay or late materialization without exact handle-domain proof.
- Still rejected: per-query evidence reconstruction, independent Bag multiplicity/order cache, persistent root per nested certificate node, fresh Group output root, SQL fallback, source-A inverse/provenance current routing, migration progress journal, Context-as-auth, host callback validation.

### PERFORMANCE BASELINES TO PRESERVE

- P418 Bag witness publication warm: **~0.005–0.007 ms / 100k** vs dense **~4.97–6.27 ms**.
- P417 Set witness publication warm: **~0.005–0.009 ms / 100k**.
- P415 nested certificate tree: **~0.832–0.896x** baseline.
- P414 shared Scan self-Join late warm: **~1.095–1.097x**.
- P412 Join: **~0.807–0.878x**; P411 AntiJoin: **~0.882–0.979x**.
- P407 hot B-write prepare ~5.4 us; root clone ~2.5–2.9 us; overlay install ~11.9–13.2 us.

P419 makes no new throughput claim: it removes a duplicated owner while preserving the witness-owned O(1)-style seed-view cost class established in P417/P418.

### LEDGER CARRY RULE

Every successor PASS report/master handoff must carry all unresolved Context/Auth/Semantic Rules/history/bindings/durability/product lines. A line disappears only when explicitly CLOSED, SUPERSEDED, or REJECTED with concrete replacement/reason.

### NEXT RECOMMENDED PASS

PASS420 should hostile retained-root memory under many pinned snapshots/watch anchors and then attack factorized/streaming general preparation + measured workload-weighted overlay compaction. Do not introduce a second historical physical store while doing so.

### PROVISIONAL PASS FORECAST — NOT A COMMITMENT

- **P420:** retained-root memory/reclamation + factorized/streaming global preparation and compaction economics.
- **P421:** close any remaining maintained-position/history-root ownership seams exposed by P420 and finalize current realization/runtime retention contract.
- **P422+:** durable PhysicalAtoms / durable `RealizationRoot` + historical root reachability if P420/P421 remain clean.

Hostile/R&D evidence may reorder or reject this forecast.

## PASS420 — retained witness-node reclamation + certified compaction rebind

### CLOSED THIS PASS

- Added exact structural-node retention diagnostics to `kernel-persistent` and `RelationBaseWitness`: map/vector node counts, pointer-identity sharing counts, and weak unique-node probes that do **not** keep old storage alive.
- Set hostile: 65 pinned 4,096-row witnesses retain **19,424** structural nodes versus **1,064,960** for naïve full copies (`0.018239x`); nodes unique to the dropped oldest witness are reclaimed immediately.
- Bag FIFO hostile: 65 pinned 4,096-row witnesses retain **6,200** nodes versus **278,720** naïve (`0.022245x`), including FIFO occurrence-bucket vector storage; oldest-unique nodes are reclaimed after drop.
- Runtime hostile: 33 pinned immutable runtime snapshots retain **17,920** witness structural nodes versus **540,672** naïve (`0.033144x`); dropping the oldest snapshot releases its witness-unique persistent nodes.
- Factorized relation compaction no longer rebuilds `RelationBaseWitness` by recanonicalizing every materialized row. It rebinds certified canonical keys from the existing dense physical `RelationScanOccurrenceSeed` onto the new dense generation-zero handle domain.
- Hostile-rejected an incorrect witness-logical-order rebind: factorized overlays use physical `swap_remove`, so semantic survivor-order is not necessarily physical row-order. The accepted rebind is explicitly **scan-seed driven**.
- Same-run 100k/depth-1024 witness phase: certified physical-order rebind measured **0.724x / 0.806x / 0.845x** the old `RelationBaseWitness::build` cost across repeated release runs. No universal compaction threshold is inferred from these measurements.

### OPEN IMMEDIATE — CARRY FORWARD

1. Extend retention accounting from relation-witness persistent nodes to the **whole pinned runtime/history/watch root** (Revision/model, PhysicalStore, maintained materializations and history anchors) before durable physical-root retention is introduced.
2. Finish workload-weighted overlay compaction policy. Keep the P405 law `sum(H_i * ΔC_i) >= C_compact`; do not replace it with fixed depth/N thresholds. P420 reduced witness rebuild cost but did not choose a product policy.
3. Resume factorized/streaming global `RelExpr` preparation debt: avoid materializing/re-reading complete row vectors where execution certificates or columnar/streaming owners can supply the same physical preparation directly.
4. Audit maintained `CanonicalRowPositionIndex` construction/update for duplicate canonicalization while preserving its distinct position-index role.
5. Group full result-row evidence only if incrementally derivable from maintained group + aggregate authority; fresh Group root remains rejected.
6. Durable PhysicalAtoms / durable `RealizationRoot` only after the above retention/preparation gates.

### OPEN DEFERRED — MANDATORY CARRY

- durable PhysicalAtoms + durable `RealizationRoot`; crash/root-switch matrix; single-file compaction; encrypted/authenticated atom/root metadata;
- historical `HistoricalRevisionRoot -> RealizationRoot -> PhysicalAtoms` reachability without a second history store;
- Context reference/optional-reference patches, relationship mutations, safe create/delete and final Database/Context DX;
- field-granular certified change coordinates;
- DB-owned granular authorization for query/change/history/watch/hosted sessions;
- deterministic regex/`Matches`, richer serialized Semantic Rules, entity/model invariants, transaction `require`;
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics;
- final Python/.NET/Studio surfaces, backup/restore/corruption UX, public perf/binary budgets, native Windows secure-memory expansion.

### SUPERSEDED / REJECTED — DO NOT REINTRODUCE

- REJECTED P420 prototype: dense compaction rebind from semantic stable-slot order. Physical overlay `swap_remove` order can differ; rebind must use the exact physical Scan seed.
- SUPERSEDED: compaction-side full row Γ recanonicalization solely to rebuild a storage-handle witness.
- Still superseded/rejected: bundle `relation_scan_seeds` directory, independent logical seed publication, per-query evidence reconstruction, Bag side cache, root-per-certificate-node, fresh Group output root, SQL fallback, current source-A inverse routing, migration progress journal, Context-as-auth, host callback validation.

### PERFORMANCE / RETENTION BASELINES TO PRESERVE

- P420 pinned Set witness lineage: **19,424 / 1,064,960 naïve structural nodes** across 65 roots.
- P420 pinned Bag FIFO witness lineage: **6,200 / 278,720** across 65 roots.
- P420 pinned runtime snapshots (witness nodes): **17,920 / 540,672** across 33 roots.
- P420 compaction witness rebind / legacy build: **0.724–0.845x** in repeated 100k/depth-1024 release runs.
- P418 Bag witness publication warm: ~0.005–0.007 ms / 100k; P417 Set ~0.005–0.009 ms / 100k.
- P415 nested certificate tree: ~0.832–0.896x baseline; P414 shared Scan self-Join late warm ~1.095–1.097x.

### NEXT RECOMMENDED PASS

PASS421: whole-root/watch-anchor retention census + workload-weighted compaction policy/global-preparation hostile, then maintained-position/history-root seams. Do not start durable PhysicalAtoms merely because witness-level retention is now clean.

### PROVISIONAL FORECAST — NOT A COMMITMENT

- **P421:** whole runtime/history/watch retention + weighted compaction/global preparation + maintained position-index audit.
- **P422:** close any remaining current realization/runtime/history-root ownership seams exposed by P421.
- **P423+:** durable PhysicalAtoms / durable `RealizationRoot` and historical physical-root reachability if P421/P422 stay clean.

## PASS421 — whole-root retention census + certified Γ-delta reuse + workload compaction law

### CLOSED THIS PASS

- Whole-root hostile extended P420 beyond witness nodes. In a 33-snapshot lineage over a 4,096-row Set relation with one remove+insert per revision, the immutable runtime snapshots retain **33 distinct logical relation row buffers / 135,168 logical row slots**, while the already-persistent witness union retains **17,920 structural nodes**. Therefore witness sharing is not evidence that the complete `Revision/model` root is path-copy bounded.
- Audited Watch ownership: `QueryWatch` retains the runtime only weakly and owns maintained query state/cursors, not a historical `RuntimeRevisionSnapshot`. Long-lived `ReadContext`/historical revision authorities remain the direct runtime-root pinning surface; Watch still carries causal/history obligations through its maintained state.
- Removed repeated Γ work from the storage-resolved change path. `StorageResolvedRelationDelta` now seals exact removed/inserted canonical row keys once under its `SemanticContext`; maintained Scan patching and `RelationBaseWitness` advancement consume that same certified carrier. No second canonicalization authority or fallback path was added.
- Added the product-level workload-weighted compaction decision law. Workload classes contribute signed `expected_uses * (overlay_cost - native_cost)` economics; compaction is selected iff total positive savings cover compaction cost, maintenance cost, and workloads whose native form would be more expensive. No overlay-depth/cardinality routing is encoded.
- Frozen regression: `kernel-query` **141 passed / 9 ignored**, `kernel-realization` **20 passed / 10 ignored**, `kernel-plan` **297 passed / 5 ignored**, workspace `cargo check --workspace --all-targets --offline` PASS, public cfmd **44/44**, public contract **1/1**, public API verifier PASS, derive diagnostics PASS.

### SELECTED IMPLEMENTATION / R&D LAW

```text
storage transition
    -> resolve physical handles
    -> canonicalize changed rows ONCE under Γ
    -> StorageResolvedRelationDelta { rows, handles, canonical keys, context }
        -> maintained Scan position index
        -> RelationBaseWitness

No downstream Γ rebuild for the same changed rows.
```

Compaction policy is representation-independent:

```text
positive_savings = Σ H_i * max(C_overlay_i - C_native_i, 0)
negative_penalty = Σ H_i * max(C_native_i - C_overlay_i, 0)

compact iff
positive_savings >= C_compact + C_maintenance + negative_penalty
```

The policy consumes workload/cost evidence; it does not route on arbitrary depth/N thresholds.

### OPEN — IMMEDIATE

1. **Persistent logical relation authority / whole-root reclamation.** `Revision/model` still stores a fresh full row buffer for every changed relation revision. Design one exact persistent logical representation (Set + Bag semantics, exact row order/multiplicity, migration/history compatibility) so pinned immutable revisions retain base O(N) + changed paths/rows rather than O(N * pinned revisions). Do not hide this behind lazy full-vector rematerialization/cache fallback.
2. **Factorized/streaming global `RelExpr` preparation.** Current general preparation can materialize complete source/target row vectors and may rebuild result evidence. Design a certificate-carrying source/sink law that streams/factorizes directly into physical columns/segments, with exact output occurrence evidence and no SQL/generic fallback.
3. **Maintained `CanonicalRowPositionIndex` structural ownership.** Duplicate canonicalization on the storage-resolved transition is closed, but the maintained by-key/by-position representation still owns canonical-key payloads for position semantics. Audit structural sharing/compact key ownership without collapsing its distinct position-index role into the witness.
4. Group full result evidence only if incrementally derivable from maintained group + aggregate authority; fresh Group root remains rejected.
5. Durable PhysicalAtoms / durable `RealizationRoot` only after the logical whole-root retention and global-preparation gates are closed.

### OPEN — DEFERRED / MANDATORY CARRY

- durable PhysicalAtoms + durable `RealizationRoot`; crash/root-switch matrix; single-file compaction; encrypted/authenticated atom/root metadata;
- historical `HistoricalRevisionRoot -> RealizationRoot -> PhysicalAtoms` retention without a second history store;
- Context reference/optional-reference patches, relationship mutations, safe create/delete and final Database/Context DX;
- field-granular certified change coordinates;
- DB-owned granular authorization for query/change/history/watch/hosted-session coordinates;
- deterministic regex/`Matches`, richer serialized Semantic Rules, entity/model invariants, transaction `require`;
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics;
- final Python/.NET/Studio surfaces, backup/restore/corruption UX, public perf/binary budgets, native Windows secure-memory expansion.

### SUPERSEDED / REJECTED — DO NOT REINTRODUCE

- SUPERSEDED: recanonicalizing a storage-resolved changed row independently in maintained Scan and again in `RelationBaseWitness`; one sealed Γ carrier is now authoritative for that transition.
- REJECTED: inferring whole-root historical memory from witness-node sharing alone; the logical `Revision/model` row buffer is a separate retained owner and P421 measured it directly.
- REJECTED: fixed overlay-depth / fixed-N compaction thresholds; workload-weighted economics is the universal policy law.
- Still rejected: runtime seed directory, independent logical seed publication, per-query evidence rebuild, Bag side cache, nested persistent certificate roots, fresh Group root, SQL fallback, source-A inverse current routing, migration progress journal, Context-as-auth, host callback validation.

### PERFORMANCE / RETENTION BASELINES TO PRESERVE

- P421 whole-root hostile: 33 pinned snapshots × 4,096 rows = **135,168 logical row slots** in **33 unique logical buffers**; witness structural union = **17,920 nodes**.
- P420 witness-only 33-snapshot ratio: **3.3144%** of naïve full witness-node copies; this remains valid but is explicitly narrower than whole-root memory.
- P420 certified compaction witness rebind / legacy rebuild: **0.724x / 0.806x / 0.845x**.
- Earlier native-cost-class realization/segment baselines remain mandatory; P421 adds no hot-read regression path.

### LEDGER CARRY RULE

Every successor PASS report/master handoff must carry all unresolved Context/Auth/Semantic Rules/history/bindings/durability/product lines. A line disappears only when explicitly CLOSED, SUPERSEDED, or REJECTED with concrete replacement/reason.

### NEXT RECOMMENDED PASS

**PASS422:** attack the newly exposed full logical-row retention owner. Replace or refactor `kernel-model` relation storage so immutable revisions structurally share unchanged relation data while preserving exact Set/Bag order, Γ semantics, history and change laws; prove reclamation with pinned whole runtime roots. Then return to factorized/streaming global preparation. Durable PhysicalAtoms remain gated on this result.

## PASS422 — persistent logical delta roots + compatibility-projection hostile

### CLOSED THIS PASS
- Introduced a CFMD-native persistent logical relation authority: immutable materialized base + persistent sparse removed-base-position set + persistent ordered inserted tail. Successor Set/Bag endpoints path-copy delta structures rather than owning a new full O(N) logical row vector.
- Runtime-derived relation targets now preserve the exact existing Γ/RelationDelta endpoint and rebind that endpoint to persistent logical storage; no inverse/source-A routing or SQL/generic fallback was introduced.
- `kernel-model` validation/live-ref consumers added transient owned materialization paths so several semantic checks no longer require turning the persistent root into the authoritative full vector.
- P422 hostile, 33 pinned snapshots × 4,096 rows with one remove+insert per revision: naïve full-copy authority = 135,168 row slots; persistent delta authority upper bound = **528 owned delta rows + 225 persistent structural delta nodes**; P420 witness union remains 17,920 nodes.
- Frozen gates: `kernel-model` 10/10, `kernel-revision` 6/6, `kernel-plan` 297/297 non-ignored + 5 ignored; workspace `--all-targets` check PASS.

### SELECTED IMPLEMENTATION / R&D LAW
```text
logical relation revision R0:
    immutable base rows B

successor Rt:
    B
    - persistent sparse removed-base-position set Dt
    + persistent ordered inserted tail It

materialize(Rt) = survivors(B, Dt) ++ It

successor patch:
    (Dt, It) --exact removed survivor positions + inserts--> (Dt+1, It+1)
```
Authority is the factorized root. Full contiguous rows are a compatibility projection, never a second semantic law.

### OPEN — IMMEDIATE
1. **Eliminate strong compatibility projection retention.** Hostile found the old vector-like `RelationStore::get` / `SharedRelationRows::Deref` surface can lazily cache a full materialized vector inside an already-published persistent root. In the 33-snapshot hostile, 32/33 roots acquired such a projection during subsequent runtime work. This means the new authority is path-copy bounded, but whole-runtime memory is NOT yet closed. Replace the borrowed-Vec façade with explicit transient/guarded iteration/materialization that cannot become retained authority; do not solve this with cache eviction heuristics.
2. Prove whole-runtime/history/read-context reclamation again after the borrowed-Vec façade is removed.
3. Factorized/streaming global `RelExpr` preparation with exact occurrence/result certificates; no generic fallback.
4. Continue structural ownership audit for maintained `CanonicalRowPositionIndex`.
5. Durable PhysicalAtoms / durable `RealizationRoot` remains gated on items 1–3.

### OPEN — DEFERRED / MANDATORY CARRY
- durable PhysicalAtoms + durable `RealizationRoot`; crash/root-switch matrix; single-file compaction; encrypted/authenticated atom/root metadata;
- historical `HistoricalRevisionRoot -> RealizationRoot -> PhysicalAtoms` retention without a second history store;
- Context reference/optional-reference patches, relationship mutations, safe create/delete and final Database/Context DX;
- field-granular certified change coordinates;
- DB-owned granular authorization for query/change/history/watch/hosted-session coordinates;
- deterministic regex/`Matches`, richer serialized Semantic Rules, entity/model invariants, transaction `require`;
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics;
- final Python/.NET/Studio surfaces, backup/restore/corruption UX, public perf/binary budgets, native Windows secure-memory expansion.

### SUPERSEDED / REJECTED
- SUPERSEDED: fresh full logical row vector as the normal authority for a relation-only successor produced by runtime-derived exact deltas.
- REJECTED: calling the retained-memory problem closed merely because the factorized authority is small; a strong compatibility projection is still retained memory.
- REJECTED: eviction/LRU/fixed-depth tricks as a substitute for removing the borrowed-Vec compatibility authority.
- Carry all prior rejects: SQL fallback, source-A inverse current routing, migration progress journal, Context-as-auth, host callback validation, per-query evidence rebuild, fresh Group root.

### PERFORMANCE / RETENTION BASELINES TO PRESERVE
- P422 persistent logical authority: 33 snapshots, 4,096-row base, one remove+insert each revision: **528 delta-row upper bound + 225 persistent delta structural nodes** vs **135,168 naïve full-copy row slots**.
- P422 compatibility hostile: **32/33** roots can still hold a strong full-vector projection after later runtime work; therefore this is the immediate blocker.
- P421/P420 witness and realization native-cost-class baselines remain in force.

### NEXT RECOMMENDED PASS
**PASS423:** remove the vector-borrow compatibility/cache seam from persistent logical relations, migrate semantic/query/transport consumers to explicit iteration/transient materialization contracts, then rerun whole pinned runtime/history reclamation. Only after that resume global `RelExpr` preparation and durable roots.

## PASS423 — borrowed-Vec compatibility authority removed + whole-root reclamation closure

### CLOSED THIS PASS

- Continued directly from the frozen/unshipped P422 checkpoint. The P422 persistent logical authority remains `immutable base - persistent removed-base positions + persistent inserted tail`.
- Removed the strong lazy contiguous compatibility cache from `SharedRelationRows`. Persistent relation roots no longer contain a `OnceLock<Vec<Row>>` and immutable reads cannot mutate a historical root into an O(N) retained projection.
- Removed `SharedRelationRows::Deref<Target = Vec<Row>>`. Relation consumers now use exact persistent `iter()` / indexed access, or an explicit transient `materialize_owned()` only at boundaries that genuinely require a contiguous owned row vector (legacy exact evaluator inputs, checkpoint encoding, recovery/native row-store construction, migration materialization).
- Mutable compatibility operations remain explicit and local to a mutable owner: `RelationStore::get_mut` / `SharedRelationRows::push` may materialize the candidate being mutated, but cannot install a cache into an immutable published snapshot.
- Added exact logical-root weak reclamation evidence. `PersistentOrdSet` now exposes its existing map-node weak probe, and `SharedRelationRows` composes removed-set + inserted-tail probes. Dropping the only owning historical snapshot reclaims all probed path-copied logical delta nodes.
- Whole-root hostile now deliberately calls `to_vec()` over every persistent historical logical root before checking retention. The observed projection state remains `[true, false, ... false]`: only the original materialized base is materialized; every successor remains persistent after reads.
- Retention numbers are preserved: 33 pinned snapshots × 4,096 rows = 135,168 naïve full-copy row slots versus 528 owned delta-row upper bound + 225 persistent logical delta nodes; relation witness union remains 17,920 structural nodes.
- Relation candidate dangling-live-ref cleanup no longer clones/materializes the complete relation on the common no-dangling path; it scans the persistent view and allocates a replacement only when filtering is actually necessary.

### SELECTED IMPLEMENTATION / R&D LAW

```text
published logical relation root
    = immutable authority
    = Materialized(base) OR DeltaRoot(base, persistent removals, persistent tail)

immutable read
    -> borrowed persistent iterator / point access
    -> no retained projection mutation

explicit materialization boundary
    -> owned Vec<Row>
    -> transient caller-owned value
    -> NEVER stored back into an immutable root as a read cache

mutable compatibility boundary
    -> materialize only the mutable COW candidate
    -> old published roots remain unchanged
```

This removes the compatibility fallback as an authority. There is one logical representation law plus explicit lowering/materialization at consumers that require contiguous ownership.

### HOSTILE / PERFORMANCE CONCLUSION

- The P422 blocker `32/33 historical roots acquire strong full-vector projections` is CLOSED. Successor roots cannot acquire such a projection because no cache exists.
- Persistent iteration is structural and allocation-free. Point access resolves directly against base survivors / inserted tail. Algorithms that still fundamentally request complete `Vec<Row>` now make that O(N) cost visible at their own boundary rather than silently retaining it in history.
- Weak logical-delta probes prove reclamation, not only low accounting numbers.
- P423 does not claim arbitrary long-lived delta chains need no representation compaction. P421's workload-weighted compaction law remains the correct universal policy; fixed depth/N thresholds remain rejected.

### OPEN — IMMEDIATE

1. **Factorized/streaming global `RelExpr` preparation.** `prepare_general_relation_factorized` still constructs complete source relation row vectors (`rule.evaluate_rows`) and eventually complete target rows/columns. R&D a certificate-carrying source/sink execution law that can consume factorized/segment owners and emit physical columns + exact occurrence evidence without an intermediate whole-row model where the algebra permits it. No generic SQL fallback or operator-name routing.
2. **Maintained `CanonicalRowPositionIndex` structural ownership.** Transition duplicate Γ computation is already closed; now audit structural sharing/key payload ownership while keeping position semantics distinct from witness occurrence identity.
3. **Group full result evidence** only if derivable incrementally from maintained group + aggregate authority; fresh Group root remains rejected.
4. After 1–3 are clean, proceed to durable PhysicalAtoms / durable `RealizationRoot` and historical physical-root reachability.

### OPEN — DEFERRED / MANDATORY CARRY

- durable PhysicalAtoms + durable `RealizationRoot`; crash/root-switch matrix; single-file compaction; encrypted/authenticated atom/root metadata;
- historical `HistoricalRevisionRoot -> RealizationRoot -> PhysicalAtoms` retention without a second history store;
- Context reference/optional-reference patches, relationship mutations, safe create/delete and final Database/Context DX;
- field-granular certified change coordinates;
- DB-owned granular authorization for query/change/history/watch/hosted-session coordinates;
- deterministic regex/`Matches`, richer serialized Semantic Rules, entity/model invariants, transaction `require`;
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics;
- final Python/.NET/Studio surfaces, backup/restore/corruption UX, public perf/binary budgets, native Windows secure-memory expansion.

### SUPERSEDED / REJECTED

- SUPERSEDED: immutable `SharedRelationRows` borrowed-`Vec`/`Deref` API backed by a strong lazy full-vector cache.
- REJECTED: cache eviction/LRU/fixed-depth policy as a substitute for removing the compatibility authority.
- REJECTED: implicit materialization hidden behind an immutable read. Contiguous materialization must now be explicit at the consumer boundary.
- Carry prior rejects: SQL fallback, source-A inverse current routing, migration progress journal, Context-as-auth, host callback validation, per-query Γ evidence rebuild, fresh Group root, second physical Γ authority.

### PERFORMANCE / RETENTION BASELINES TO PRESERVE

- P423 whole-root persistent logical authority: **528 delta-row upper bound + 225 persistent delta structural nodes** vs **135,168 naïve row slots** across 33×4,096 roots.
- P423 successor projection retention after explicit reads: **0/32 persistent successor roots** retain a full compatibility projection.
- P423 weak probe: oldest persistent delta's unique logical nodes are fully reclaimed after its snapshot is dropped.
- P420 witness union remains **17,920 nodes** across the same 33-root runtime lineage.
- P420/P421 realization/compaction/native-cost-class baselines remain mandatory.

### LEDGER CARRY RULE

Every successor PASS report/master handoff must carry all unresolved Context/Auth/Semantic Rules/history/bindings/durability/product lines. An item disappears only when explicitly CLOSED, SUPERSEDED, or REJECTED with concrete replacement/reason.

### NEXT RECOMMENDED PASS

**PASS424:** hostile/R&D `prepare_general_relation_factorized` as a streaming/factorized certificate pipeline. The target is to remove avoidable `source physical columns -> full rows -> RelExpr -> full target rows -> split back to columns` staging without introducing a fallback path. Measure memory/work against cardinality/dependency width and preserve exact output occurrence evidence. Durable PhysicalAtoms remain immediately after this preparation/position-index gate, not before it.

## PASS424 — global preparation hostile: maintained streaming REJECTED

### CLOSED THIS PASS
- Hostile-tested a no-fallback universal streaming prototype for `prepare_general_relation_factorized`: factorized source rows were fed in bounded exact deltas through `MaterializedRelPlanState`, and exact output deltas were consumed by a transient physical sink with witness/physical-Scan evidence.
- Semantic/retraction behavior worked, including non-append output deltas; the prototype therefore answered the correctness question but failed the product performance gate.
- 100k Bag Union release: accepted P423 preparation **311.676 ms**; full maintained-streaming prototype **3199.062 ms** (~10.3x); with the target sink temporarily disabled **1907.967 ms** (~6.1x). Dominant cost is rebuilding maintained Scan/persistent position authority, not output columnization.
- Prototype production source changes were fully reverted. No slow fallback/router was retained.
- Selected the next architecture: a storage-neutral one-shot `RelExecutionSource -> operator DAG -> RelExecutionSink` law that can consume factorized/segment storage and existing Γ Scan evidence without constructing long-lived maintained Scan authority.

### SELECTED IMPLEMENTATION / R&D LAW
```text
one-shot global preparation
    != maintained Query/Watch bootstrap

Prepared RelExpr
+ storage-neutral factorized source/range visitor
+ existing exact Γ evidence where transportable
    -> one-shot operator state only
    -> physical column/segment sink
    -> exact physical-order output occurrence evidence
```

Working memory target:
```text
O(batch + inherent operator state + output authority)
```
not full source model + maintained Scan authority + target rows + target columns.

### OPEN — IMMEDIATE
1. Implement the one-shot relational execution source/sink boundary in `kernel-query`/`kernel-realization`; Scan must consume factorized/range input without owning a future-update `CanonicalRowPositionIndex` merely for preparation.
2. Lower the existing `RelExpr` DAG onto that boundary. Per-operator implementations are allowed under the single execution law; missing exact lowerings fail closed and MUST NOT call the old whole-row evaluator as fallback.
3. Reuse P408–P415 execution certificates / P414–P420 Scan evidence so unchanged source rows and final target occurrence evidence are not Γ-canonicalized twice.
4. Re-run 100k Bag Union plus Set Difference/Join/AntiJoin/Group hostile. Any accepted one-shot path must beat or remain in the accepted current cost class while materially reducing peak whole-row staging.
5. Audit maintained `CanonicalRowPositionIndex` ownership only after it is no longer being proposed as the one-shot preparation substrate.
6. Durable PhysicalAtoms / durable `RealizationRoot` remain gated until the one-shot global-preparation boundary is accepted.

### OPEN — DEFERRED / MANDATORY CARRY
- durable PhysicalAtoms + durable `RealizationRoot`; crash/root-switch matrix; single-file compaction; encrypted/authenticated atom/root metadata;
- historical `HistoricalRevisionRoot -> RealizationRoot -> PhysicalAtoms` retention without a second history store;
- Context reference/optional-reference patches, relationship mutations, safe create/delete and final Database/Context DX;
- field-granular certified change coordinates;
- DB-owned granular authorization for query/change/history/watch/hosted-session coordinates;
- deterministic regex/`Matches`, richer serialized Semantic Rules, entity/model invariants, transaction `require`;
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics;
- final Python/.NET/Studio surfaces, backup/restore/corruption UX, public perf/binary budgets, native Windows secure-memory expansion.

### SUPERSEDED / REJECTED
- REJECTED P424: bootstrap one-shot migration preparation by replaying bounded source batches through `MaterializedRelPlanState`; semantically exact but ~10.3x accepted 100k Bag Union preparation.
- REJECTED: interpreting a fixed source batch size as the solution; the retained payer is maintained Scan authority itself.
- Still rejected: generic SQL/row fallback, source-A current routing, independent Γ caches, per-query evidence rebuild, fixed compaction depth/N, Context-as-auth, host callback validation.

### PERFORMANCE BASELINES TO PRESERVE
- P424 accepted P423 baseline, 100k Bag Union: **311.676 ms** preparation; cutover **75.441 us**.
- P424 rejected maintained streaming: **3199.062 ms**; no-target-sink control **1907.967 ms**.
- Earlier P408–P415 certificate/native-cost-class baselines remain mandatory.

### NEXT RECOMMENDED PASS
**PASS425:** implement the one-shot storage-neutral relational execution source/sink boundary and first exact lowerings without maintained Scan ownership or fallback. Validate Bag Union and a retracting Set/Difference-class query before broadening to Join/Group/TopK.

## PASS425 — one-shot RelExecution accepted: no maintained bootstrap, no whole-row fallback

### CLOSED THIS PASS
- Replaced `prepare_general_relation_factorized`'s complete-source `FiniteModel` + full target-row staging with a storage-neutral one-shot execution boundary.
- Added `RelExecutionSource` over factorized physical relation authority and `RelExecutionSink` whose production owner is a columnar target sink.
- Target `RelationBaseWitness` is built directly with `RelationBaseWitness::build_columnar`; target rows are never reconstructed merely to rebuild Γ evidence.
- Exact lowerings now exist for `Scan`, `Union`, `Difference`, and `AntiJoin`. Bag `Difference` owns only Γ blocker multiplicities; `AntiJoin` owns only Γ blocker-key support. Unsupported operators fail closed with `UnsupportedOneShotRelExpr` and do not invoke the old whole-row evaluator.
- Direct `Bag Union(Scan, Scan)` has a factorized column-range lowering under the same execution law, so it transfers physical/factorized columns into the columnar sink without transient `Row` assembly.
- Added hostile regressions for Bag Difference multiplicity subtraction, AntiJoin complete-fiber blocking, and explicit no-fallback failure for an unsupported Filter lowering.

### SELECTED IMPLEMENTATION / R&D LAW
```text
verified RelExpr
+ RelExecutionSource
    -> factorized relation row/column range access
+ one-shot operator lowering
    -> only mathematically inherent state
+ RelExecutionSink
    -> target physical columns
    -> RelationBaseWitness::build_columnar
    -> exact physical Scan seed
```

Operator-specific algorithms are lowerings of this one execution law, not semantic routers. A missing lowering is an explicit proof/implementation gap, not permission to revive full-model evaluation.

### HOSTILE / PERFORMANCE CONCLUSION
100k Bag Union, release, warm separate runs:
- P423 accepted whole-row preparation baseline: **311.676 ms**.
- P424 rejected maintained-streaming prototype: **3199.062 ms**; no-target-sink control **1907.967 ms**.
- P425 accepted one-shot + direct column-range Union: **284.239 / 265.027 / 245.619 ms**.

The important closure is structural: no complete source `FiniteModel`, no complete target `Vec<Row>`, no long-lived maintained Scan/position authority, and no output-row recanonicalization after columnization. Timing remains a sandbox microbenchmark, not a product constant.

### OPEN — IMMEDIATE
1. Extend the one-shot DAG with exact lowerings for Filter/Project/Set Union/Distinct, then Join; reuse existing P408–P415 certificate laws where they eliminate duplicate Γ work.
2. Group/TopK one-shot lowerings only with their mathematically inherent annotation/order state; do not bootstrap `MaterializedRelPlanState`.
3. Make occurrence/canonical evidence flow through the one-shot source/sink where derivable, especially Set paths, so `build_columnar` does not recanonicalize output that already has exact Γ evidence.
4. Audit maintained `CanonicalRowPositionIndex` structural/key ownership now that it is no longer proposed as the preparation substrate.
5. After 1–4 remain clean, resume durable PhysicalAtoms / durable `RealizationRoot` and historical physical-root reachability.

### OPEN — DEFERRED / MANDATORY CARRY
- durable PhysicalAtoms + durable `RealizationRoot`; crash/root-switch matrix; single-file compaction; encrypted/authenticated atom/root metadata;
- historical `HistoricalRevisionRoot -> RealizationRoot -> PhysicalAtoms` retention without a second history store;
- Context reference/optional-reference patches, relationship mutations, safe create/delete and final Database/Context DX;
- field-granular certified change coordinates;
- DB-owned granular authorization for query/change/history/watch/hosted-session coordinates;
- deterministic regex/`Matches`, richer serialized Semantic Rules, entity/model invariants, transaction `require`;
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics;
- final Python/.NET/Studio surfaces, backup/restore/corruption UX, public perf/binary budgets, native Windows secure-memory expansion.

### SUPERSEDED / REJECTED
- SUPERSEDED for supported one-shot operators: `source physical columns -> full source rows -> FiniteModel -> RelExpr -> full target rows -> split columns` preparation.
- REJECTED: fallback from an unsupported one-shot lowering to the old whole-row evaluator.
- REJECTED P424 maintained-query bootstrap remains rejected.
- Carry prior rejects: SQL fallback, source-A current routing, independent Γ caches, per-query evidence rebuild, fixed compaction depth/N, Context-as-auth, host callback validation.

### PERFORMANCE BASELINES TO PRESERVE
- P425 100k direct Bag Union one-shot: **245.619–284.239 ms** across three warm runs.
- P423 previous accepted baseline: **311.676 ms**.
- P424 rejected maintained streaming: **3199.062 ms**.
- P423/P420 persistent logical/witness retention baselines remain mandatory.

### NEXT RECOMMENDED PASS
**PASS426:** extend the accepted one-shot law through Filter/Project/Set Union/Distinct and Join while transporting exact occurrence evidence where possible; then hostile-audit `CanonicalRowPositionIndex` ownership. Do not start durable PhysicalAtoms until this operator/evidence closure is stable.

## PASS426 — one-shot Filter/Project/Set quotient/Join closure

### CLOSED THIS PASS

- Extended the accepted one-shot law to `FilterEqConst`, `FilterOrderConst`, `FilterEqColumns`, Bag/Set `Project`, `Distinct`, `JoinEq`, and `PromoteToBag`; P425 Scan/Union/Difference/AntiJoin remain unchanged.
- Set Union/Project/Distinct continue to operate in Γ-canonical row-key space rather than host equality.
- `JoinEq` owns only one exact canonical join-key fiber index and streams the opposite input; no maintained Scan/Watch state is constructed.
- Added direct hostile coverage for Set Union->Project->Distinct, Bag Join multiplicity, deep Set Filter->Union->Project->Distinct->Join composition, and explicit unsupported one-shot failure.
- 100k release Join warm one-shot runs were ~60.9–70.3 ms vs generic evaluate warm samples ~74.3–93.3 ms (sandbox measurements; not product constants).

### OPEN — IMMEDIATE

1. Introduce an opaque kernel-query-owned row-aligned Γ evidence carrier so Set Union/Project/Distinct/Difference keys already computed during execution can be adopted by the target witness without a second `build_columnar` canonicalization pass. Do not add a forgeable raw-key authority API.
2. Implement one-shot `Group` and `TopKWithTies` using only their mathematically inherent aggregate/order state; remain fail-closed until proven.
3. Hostile `CanonicalRowPositionIndex` structural/key ownership after the evidence carrier lands; remove duplicated canonical-key ownership if the persistent witness can serve the same law without degrading update complexity.
4. Consider cost-aware Join orientation only as a lowering of the same exact fiber law; no SQL planner/router semantics.
5. Durable PhysicalAtoms / durable `RealizationRoot` only after this execution/evidence gate is stable.

### OPEN — DEFERRED / MANDATORY CARRY

- durable PhysicalAtoms + durable `RealizationRoot`, crash/root-switch matrix, single-file compaction, encrypted/authenticated atom/root metadata;
- `HistoricalRevisionRoot -> RealizationRoot -> PhysicalAtoms` reachability without a second history store;
- Context reference/optional-reference patches, relationship mutations, safe create/delete, final Database/Context DX;
- field-granular certified changes;
- DB-owned granular authorization for query/change/history/watch/hosted sessions;
- deterministic regex/Matches, richer Semantic Rules, entity/model invariants, transaction `require`;
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics;
- final Python/.NET/Studio, backup/restore/corruption UX, perf/binary budgets, native Windows secure-memory expansion.

### SUPERSEDED / REJECTED

- REJECTED: routing newly supported one-shot operators back into the complete-row evaluator.
- REJECTED: a public/trusted raw canonical-key constructor as a shortcut around exact Γ evidence ownership.
- Carry prior rejects: P424 maintained-query bootstrap, SQL fallback, source-A current routing, independent Γ caches, fixed compaction depth/N, Context-as-auth, host callback validation.

### NEXT RECOMMENDED PASS

**PASS427:** opaque certified row-aligned Γ evidence transport from one-shot operators to the physical sink/witness, then `CanonicalRowPositionIndex` hostile. If clean, proceed to one-shot Group/TopK or durable physical-root gating according to the evidence result.

## PASS427 — sealed Γ evidence adoption + shared position-index key payloads

### CLOSED THIS PASS
- Added `kernel-query`-owned `RelationRowCanonicalizer` and opaque `CertifiedCanonicalRowKey`. Only the compiled exact Γ authority can create a certified row key; downstream realization/storage code cannot forge one from a raw `CanonicalRowKey`.
- `RelationOccurrenceCertificate::from_dense_certified_keys` accepts only keys issued by the same in-process canonical authority and preserves Set uniqueness / Bag multiplicity laws. Mixed authority fails closed.
- One-shot top-level Set Union, Set Project, Distinct, and Set/Bag Difference now reuse the exact canonical row keys already required by their semantic law. The resulting occurrence certificate is adopted by `RelationBaseWitness::from_occurrence_certificate`; output rows are not canonicalized again by `build_columnar`.
- Added hostile proof that certificate -> witness adoption shares the same persistent occurrence root, and that equal-looking keys from two separately compiled authorities cannot be mixed.
- `CanonicalRowPositionIndex` no longer owns a second full `CanonicalRowKey` payload per physical position. One canonical key payload is `Arc`-owned per Γ class; `by_position` keeps only shared pointers. `PersistentOrdMap` gained borrowed `get/get_key_value/contains_key` lookup so this does not require temporary key cloning.
- Hostile index tests prove every position pointer aliases the exact class-key allocation before and after swap-remove + append.

### SELECTED IMPLEMENTATION / R&D LAW
```text
operator needs full Γ row class anyway
    -> RelationRowCanonicalizer::certify_row(row)
    -> opaque CertifiedCanonicalRowKey
    -> semantic decision + emitted row use same evidence
    -> RelationOccurrenceCertificate
    -> RelationBaseWitness adopts occurrence root

NO second Γ(row)
NO trusted/raw key injection
NO second canonical-key payload per row-position index entry
```

### OPEN — IMMEDIATE
1. Extend sealed evidence propagation through pass-through/nested one-shot operators where the exact result row is unchanged, instead of only top-level quotient/subtraction lowerings.
2. Implement one-shot Group and TopKWithTies with only mathematically inherent group/order state and exact evidence where derivable; remain fail-closed until proven.
3. Hostile retained-memory/perf measurement for shared-key `CanonicalRowPositionIndex` under high-cardinality Set and low-cardinality Bag workloads; preserve O(log Γ-classes) lookup and swap-remove update complexity.
4. Consider cost-aware Join side orientation only as a lowering of the same exact fiber law; no planner/fallback semantics.
5. Durable PhysicalAtoms / durable `RealizationRoot` only after this execution/evidence line remains stable.

### OPEN — DEFERRED / MANDATORY CARRY
- durable PhysicalAtoms + durable `RealizationRoot`, crash/root-switch matrix, single-file compaction, encrypted/authenticated atom/root metadata;
- `HistoricalRevisionRoot -> RealizationRoot -> PhysicalAtoms` reachability without a second history store;
- Context reference/optional-reference patches, relationship mutations, safe create/delete, final Database/Context DX;
- field-granular certified changes;
- DB-owned granular authorization for query/change/history/watch/hosted sessions;
- deterministic regex/Matches, richer Semantic Rules, entity/model invariants, transaction `require`;
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics;
- final Python/.NET/Studio, backup/restore/corruption UX, perf/binary budgets, native Windows secure-memory expansion.

### SUPERSEDED / REJECTED
- SUPERSEDED for certified one-shot quotient/subtraction output: second `RelationBaseWitness::build_columnar` Γ pass.
- SUPERSEDED inside maintained canonical position index: owning one full canonical row-key payload in both class map and every `by_position` entry.
- REJECTED: public/trusted raw canonical-key injection, temporary cloned lookup keys, linear reverse lookup, or cache eviction as substitutes for exact ownership.
- Carry prior rejects: P424 maintained-query bootstrap, SQL fallback, source-A current routing, independent Γ caches, fixed compaction depth/N, Context-as-auth, host callback validation.

### PERFORMANCE / STRUCTURAL BASELINES TO PRESERVE
- P426 Join and P425 Bag Union one-shot baselines remain mandatory.
- Canonical position index payload law is now `O(number of Γ classes canonical payloads + number of rows Arc refs)` rather than `O(classes + rows full canonical payloads)`. For unique-key Set this halves canonical-key payload ownership from `2N` to `N`; for a 100k-row/1024-class Bag it changes payload ownership from `101024` canonical-key values to `1024` plus 100k pointer refs.
- Lookup remains ordered-map logarithmic; position removal remains swap-remove with class bucket repair.

### NEXT RECOMMENDED PASS
**PASS428:** propagate sealed Γ evidence through unchanged-row Filter/AntiJoin paths, then implement exact one-shot Group/TopK lowerings and measure shared-key position-index memory/perf. If those gates remain clean, resume durable PhysicalAtoms / durable RealizationRoot.

## PASS428 — complete current RelExpr one-shot lowering + sealed unchanged-row Γ propagation

### CLOSED THIS PASS
- Added exact one-shot `Group` using Γ group lookup plus only mathematically inherent `ExactCount` / `ExactF64Sum` aggregate state.
- Added exact one-shot `TopKWithTies` using canonical ordering buckets bounded to `K + boundary ties`; worse buckets are discarded during streaming instead of materializing/sorting the complete input.
- Sealed row-aligned Γ evidence now passes recursively through unchanged-row `FilterEqConst`, ordered Filter, `FilterEqColumns`, and `AntiJoin` whenever the child already owns a certified full-row token.
- The one-shot `RelExpr` match is now exhaustive over every current IR variant. `UnsupportedOneShotRelExpr` was removed: a future new relational operator must add an explicit lowering at compile time and cannot silently fall back to the old complete-row evaluator.
- P427 `CanonicalRowPositionIndex` shared-key ownership received a release hostile against frozen P426. Shared payloads improved both memory structure and runtime: unique 100k build/churn10k `1199.109/431.408 ms -> 524.350/97.649 ms`; 1024-class Bag-shaped fixture `1143.129/416.803 ms -> 284.426/86.803 ms`.

### SELECTED IMPLEMENTATION / R&D LAW
```text
one semantic RelExpr law
    -> exhaustive one-shot lowering
    -> only operator-inherent transient state
    -> columnar physical sink
    -> sealed Γ evidence where derivable
```
`Group` owns group/aggregate state, `TopKWithTies` owns only the exact ordered boundary, Join owns one matching fiber index, and blockers own only their exact blocker state. None owns maintained Query/Watch Scan authority.

### OPEN — IMMEDIATE
1. Begin durable `PhysicalAtoms + RealizationRoot` authority: checkpoint/reopen must serve the current semantic world directly from durable physical realization without reconstructing an eager full logical B state.
2. Define same-semantic-revision realization-root publication and crash/root-switch ordering; representation publication must not create semantic history events.
3. Carry current + retained historical realization roots through single-file reachability/compaction with encrypted/authenticated atom/root metadata.
4. Preserve the one-shot execution/evidence laws as the materializer for durable physical rewrites; do not create a second durable query/migration engine.
5. Optional optimization, not a semantic blocker: pass sealed full-row evidence through additional unchanged-row boundary cases such as TopK when its input already owns a compatible token.

### OPEN — DEFERRED / MANDATORY CARRY
- `HistoricalRevisionRoot -> RealizationRoot -> PhysicalAtoms` historical reachability without a second history store;
- Context reference/optional-reference patches, relationship mutations, safe create/delete, final Database/Context DX;
- field-granular certified changes;
- DB-owned granular authorization for query/change/history/watch/hosted sessions;
- deterministic regex/Matches, richer Semantic Rules, entity/model invariants, transaction `require`;
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics;
- final Python/.NET/Studio, backup/restore/corruption UX, public perf/binary budgets, native Windows secure-memory expansion.

### SUPERSEDED / REJECTED
- SUPERSEDED: runtime `UnsupportedOneShotRelExpr` as a normal preparation outcome for the current IR; all current variants are lowered exhaustively.
- REJECTED remains: fallback to complete-row `RelExpr::evaluate`, maintained Query/Watch bootstrap for one-shot preparation, forgeable raw Γ-key injection, SQL-shaped routing, source-A current routing, fixed compaction depth/N.

### PERFORMANCE BASELINES TO PRESERVE
- P428 shared position index, unique 100k: build **524.350 ms**, churn10k **97.649 ms**; P426 old payload-copy baseline **1199.109 / 431.408 ms**.
- P428 shared position index, 100k rows / 1024 classes: build **284.426 ms**, churn10k **86.803 ms**; P426 baseline **1143.129 / 416.803 ms**.
- P425 100k direct Bag Union one-shot **245.619–284.239 ms** and P426 Join warm cost class remain regression gates.
- P423/P420 persistent logical/witness retention laws remain mandatory.

### NEXT RECOMMENDED PASS
**PASS429:** start durable PhysicalAtoms / durable RealizationRoot. First establish the durable root record and same-semantic-revision publication/crash law while keeping full logical `Revision/DatabaseState` as verification oracle during transition; do not introduce a second current semantic world or a second history store.

## PASS429 — first durable PhysicalAtoms / direct RealizationRoot authority

### CLOSED THIS PASS
- Added a versioned `CFPR` durable physical-realization image carrying one semantic `RevisionId`, the reachable `PhysicalAtomStore`, and the direct factorized `RealizationRoot` topology. The global database/storage format version is unchanged; this is an independently versioned section payload.
- Durable encoding is fail-closed unless the realization root is direct and its physical topology is valid. Runtime witnesses, Scan state, overlays, caches and unreachable atoms are not serialized as authority.
- Reopen reconstructs `PhysicalAtomStore + FactorizedRealizationRoot` directly and structurally validates lifecycle/carrier/field/relation codec/shape without eagerly materializing `DatabaseState`.
- `SingleFileSectionInput::physical_realization` and `SingleFileContainer::read_active_factorized_realization` establish the first single-file physical-root path through the existing authenticated/encrypted `PhysicalArtifact` section.
- Same-semantic-revision hostile: a materialized replacement field atom changes physical dependencies while preserving the value and `RevisionId`; an unpublished tail remains non-authoritative, and after generation/root publication reopen selects the new realization root.
- Encrypted single-file hostile reopens the realization authority directly under AES-256-GCM-SIV.
- Hostile fixed a positional regression before freeze: relation `column_order` is preserved literally and checked for uniqueness only; semantic column IDs are never sorted into ordinal order.

### SELECTED IMPLEMENTATION / R&D LAW
```text
semantic RevisionId R
+
reachable PhysicalAtoms P
+
direct factorized RealizationRoot rho
    -> versioned PhysicalArtifact section
    -> authenticated/encrypted single-file generation
    -> root-slot publication

reopen:
    root slot -> generation -> physical image -> (P, rho)
    NO eager rho(P) -> DatabaseState requirement
```
Same-revision representation rewrite publishes a new physical root/generation but does not create a semantic revision/history event.

### OPEN — IMMEDIATE
1. Integrate the durable realization image into `DurableRevisionStore` checkpoint/publication ownership instead of the current standalone single-file section path; bind it to the authoritative checkpoint semantic revision.
2. Generalize durable root encoding beyond fully-direct roots: derived normalized expressions/chunk overlays must either be durably expressible or be materialized before publication by an explicit law; never serialize runtime witness/cache state.
3. Make physical image I/O streaming/bounded rather than requiring one in-memory section `Vec` at the first public codec boundary.
4. Carry current + retained historical realization roots through single-file compaction/reachability: `HistoricalRevisionRoot -> RealizationRoot -> PhysicalAtoms`, with one atom shareable by current and historical roots.
5. Extend crash/root-switch hostile through the actual store checkpoint/WAL publication matrix and compaction, plaintext + encrypted.

### OPEN — DEFERRED / MANDATORY CARRY
- Context reference/optional-reference patches, relationship mutations, safe create/delete, final Database/Context DX;
- field-granular certified changes;
- DB-owned granular auth for query/change/history/watch/hosted sessions;
- deterministic regex/Matches, richer Semantic Rules, entity/model invariants, transaction `require`;
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics;
- final Python/.NET/Studio, backup/restore/corruption UX, public perf/binary budgets, native Windows secure-memory expansion.

### SUPERSEDED / REJECTED
- REJECTED: serializing `RelationBaseWitness`, Scan seeds, maintained query state or other reconstructible runtime caches as part of durable physical authority.
- REJECTED: reopening a durable physical root by first decoding/eagerly rebuilding a full logical `DatabaseState`.
- REJECTED: sorting stable semantic relation-column IDs to derive column ordinal order.
- Carry prior rejects: second semantic/history store, source-A current routing, SQL fallback, complete-row preparation fallback, forgeable Γ evidence, fixed compaction depth/N.

### NEXT RECOMMENDED PASS
**PASS430:** integrate current durable realization authority into `DurableRevisionStore` checkpoint/single-file publication and recovery, binding its embedded semantic revision to the actual durable checkpoint/head; then carry the same root through checkpoint rotation/compaction crash matrices. Historical root/atom reachability follows immediately after current-root store ownership is exact.

## PASS430 — DurableRevisionStore owns checkpoint realization authority

### CLOSED THIS PASS
- `DurableRevisionStore` now owns optional `checkpoint_realization` authority rather than leaving CFPR as a standalone single-file helper path.
- `DurableFactorizedRealization` binds one exact semantic `RevisionId` to a validated direct `PhysicalAtomStore + FactorizedRealizationRoot`; reopen rejects a physical section whose embedded revision differs from the checkpoint cut.
- `rotate_checkpoint_with_factorized_realization(...)` publishes the physical root in the same single-file generation/root switch as checkpoint + metadata; ordinary same-revision rotation preserves the already-published physical authority.
- Single-file compaction preserves the active physical realization section and reopen restores it into the store.
- Streaming checkpoint has an explicit factorized-realization entry point. CFPR bytes are captured with the immutable checkpoint cut, while later WAL commits remain carried WAL authority. Hostile proves `checkpoint_realization.revision = R0` can coexist exactly with recovered `durable_head = R2`.
- Existing single-file encryption/authentication and root-slot publication laws apply because the physical image is a normal authenticated generation section.
- Directory explicit physical-root publication is intentionally fail-closed until it has an equivalent atomic carrier/checksum law; no backend silently discards a requested physical authority.

### SELECTED IMPLEMENTATION / R&D LAW
```text
DurableRevisionStore
    checkpoint: Revision Rcut
    checkpoint_realization: optional CFPR(Rcut)
    WAL: exact effects Rcut -> Rhead

single-file publish:
    checkpoint(Rcut)
    + metadata
    + prepared capsule
    + optional CFPR(Rcut)
    + carried WAL
        -> one generation/root switch

reopen:
    CFPR.revision MUST == checkpoint.id
    physical root is authority for checkpoint cut
    WAL remains authority from cut to durable head
```

### OPEN — IMMEDIATE
1. Give directory backend an equivalent generation-owned physical-realization carrier with exact checksum/publication ordering; until then explicit directory physical-root publication remains fail-closed.
2. Eliminate the first-boundary in-memory CFPR `Vec` by adding bounded/streaming physical-image section encoding.
3. Extend store-level physical-root crash hostile to encrypted root-switch/compaction fault matrices, not just inheritance from generic section publication tests.
4. Evolve retention to `HistoricalRevisionRoot -> RealizationRoot -> PhysicalAtoms` with shared atom reachability and no second history store.
5. Generalize durable realization beyond fully-direct roots only by a proved durable expression law or explicit pre-publication materialization; never persist runtime witness/cache state.

### OPEN — DEFERRED / MANDATORY CARRY
- Context reference/optional-reference patches, relationship mutations, safe create/delete, final Database/Context DX;
- field-granular certified changes;
- DB-owned granular authorization for query/change/history/watch/hosted sessions;
- deterministic regex/Matches, richer Semantic Rules, entity/model invariants, transaction `require`;
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics;
- final Python/.NET/Studio, backup/restore/corruption UX, public perf/binary budgets, native Windows secure-memory expansion.

### SUPERSEDED / REJECTED
- SUPERSEDED: standalone CFPR section ownership outside `DurableRevisionStore` for the active checkpoint path.
- REJECTED: treating a checkpoint physical root as automatically representing a newer WAL head.
- REJECTED: silently dropping requested physical authority on a backend without a publication law.
- Carry prior rejects: eager logical reconstruction as physical reopen path, second semantic/history store, runtime witness/cache serialization, source-A current routing, SQL fallback.

### PERFORMANCE / SAFETY BASELINES TO PRESERVE
- P429 direct reopen without eager `DatabaseState` reconstruction.
- P428/P427 one-shot Γ evidence and shared position-index laws remain the materializer/physical-maintenance baseline.
- Streaming publication must preserve exact cut + carried-WAL endpoint equality; physical realization is cut-bound, never head-guessed.

### NEXT RECOMMENDED PASS
**PASS431:** add the directory generation carrier for checkpoint realization and then start historical `RealizationRoot -> PhysicalAtoms` reachability. In parallel, replace the in-memory CFPR section buffer with bounded streaming encode so both backends share one physical-image publication law.

## PASS431 — backend-neutral checkpoint realization carrier + bounded CFPR I/O

### CLOSED THIS PASS
- Directory generations can now publish the same checkpoint-bound `DurableFactorizedRealization` authority as the single-file backend; explicit directory physical-root publication is no longer fail-closed.
- Directory CFPR is a generation-owned `realization-<generation>.cfpr` prerequisite. It is streamed and fsync'd before metadata/manifest publication; metadata codec v17 binds exact checkpoint revision, encoded length and CRC32C, and the metadata file remains checksum-bound by the manifest.
- Reopen treats a bound realization as mandatory authority: missing, truncated, checksum-mismatched or revision-mismatched CFPR is corruption rather than absence/fallback.
- Directory external-freshness generation digest now includes the realization file, and directory generation compaction recognizes/removes obsolete CFPR files while preserving pinned generations.
- Directory streaming checkpoint snapshots the realization at the immutable checkpoint cut, writes/binds/verifies it before manifest publication, and preserves the same `checkpoint_realization = Rcut`, `durable_head = Rhead` carried-WAL law as single-file.
- CFPR encoding now has exact counting + bounded streaming sinks. Production single-file synchronous/streaming publication uses a `SingleFileSectionSource`; directory publication streams directly to file. Production reopen on both backends decodes from bounded readers rather than first allocating a full CFPR `Vec`.
- Byte/length equivalence between retained reference encoding and streaming encoding is hostile-tested; emitted codec chunks are bounded to 64 KiB.

### SELECTED IMPLEMENTATION / R&D LAW
```text
checkpoint realization Rcut
    -> one canonical CFPR stream

single-file:
    CFPR stream -> authenticated/encrypted generation section -> root switch

directory:
    CFPR stream -> generation-owned realization-N.cfpr + fsync
                -> metadata binding {Rcut,len,crc32c} + fsync
                -> directory sync
                -> manifest rename + directory sync

reopen:
    binding present => CFPR is mandatory and exact
    no "file missing => fallback to logical rebuild"
```

### OPEN — IMMEDIATE
1. Begin `HistoricalRevisionRoot -> RealizationRoot -> PhysicalAtoms` reachability using the same atom graph; retained history must not become a second physical/history store.
2. Carry historical physical-root reachability through directory + single-file generation compaction and explicit release.
3. Extend physical-root-specific destructive fault matrices around directory CFPR prerequisite publication and encrypted single-file root switch/compaction.
4. Generalize durable non-direct realization only where an exact durable expression law is proved; otherwise representation-rewrite/materialize before publication.

### OPEN — DEFERRED / MANDATORY CARRY
- Context reference/optional-reference patches, relationship mutations, safe create/delete, final Database/Context DX;
- field-granular certified changes;
- DB-owned granular authorization for query/change/history/watch/hosted sessions;
- deterministic regex/Matches, richer Semantic Rules, entity/model invariants, transaction `require`;
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics;
- final Python/.NET/Studio, backup/restore/corruption UX, public perf/binary budgets, native Windows secure-memory expansion.

### SUPERSEDED / REJECTED
- SUPERSEDED: directory backend fail-closed solely because it lacked a CFPR generation carrier.
- SUPERSEDED: production CFPR publication/reopen that requires a whole encoded section `Vec`.
- REJECTED: optional best-effort directory CFPR discovered only by filename presence.
- REJECTED remains: eager logical reconstruction as physical reopen path, second semantic/history store, runtime witness/cache serialization, source-A current routing, SQL fallback.

### NEXT RECOMMENDED PASS
**PASS432:** historical physical-root reachability. Introduce the smallest root authority that binds retained historical semantic revisions to realization roots over shared PhysicalAtoms, then make GC/compaction derive atom liveness from current + retained historical roots. Do not create a second atom store or migration-progress journal.

## PASS432 — shared historical RealizationRoot authority over one PhysicalAtomStore

### CLOSED THIS PASS
- Durable CFPR authority can now retain multiple historical realization roots over the **same immutable `PhysicalAtomStore`** as the current checkpoint root; no second atom/history store is introduced.
- `DurableHistoricalRealizationRoot` binds a causal `RevisionEffectId` to its exact historical `RevisionId` and direct `FactorizedRealizationRoot` topology. Historical roots carry no duplicate atom payloads.
- CFPR envelope v2 encodes the union of atoms reachable from the current root plus retained historical roots, followed by current and historical root topologies. Decoder remains fail-closed and accepts v1 current-only images for continuity.
- `DurableFactorizedRealization::retain_historical_root` merges atom lineages by immutable `PhysicalAtomId`; same ID with different payload is rejected rather than remapped or copied under another authority.
- `inherit_retained_historical_roots` lets synchronous and streaming factorized checkpoint publication carry forward exactly the roots named by live `HistoricalEpochAnchor`s. A new migration boundary can retain the previous checkpoint root when its `source_revision` matches.
- `release_historical_root` removes the causal root and immediately prunes atoms unreachable from `current ∪ retained history`. `release_historical_epoch_authority` now performs the same physical-root release transactionally and restores the previous realization on a pre-publication failure.
- Store observation can resolve a retained causal effect directly to `(historical RevisionId, shared PhysicalAtomStore, historical FactorizedRealizationRoot)` without opening another physical store.
- Hostile unit proof: old and current roots share exact atom IDs; CFPR roundtrip preserves the historical root; release removes the old-only atom while preserving shared/current atoms.

### SELECTED IMPLEMENTATION / R&D LAW
```text
DurableFactorizedRealization
    shared PhysicalAtomStore P
    current root rho_current : P -> current semantic world
    historical roots {
        effect_id -> (historical_revision, rho_historical)
    }

reachable(P) = deps(rho_current)
             U union deps(rho_historical for retained effects)

release(effect):
    remove rho_effect
    P := retain(P, reachable(P))
```

Physical atom identity is immutable lineage identity. A current realization freshly allocating the same `PhysicalAtomId` for a different payload is **not** merge-compatible; the operation fails closed instead of renumbering/remapping atoms and silently creating a second identity law.

### OPEN — IMMEDIATE
1. Replace the conservative generation-level `HistoricalEpochMaterial` path when a retained historical realization root is sufficient: bind historical semantic registry/context + exact causal WAL boundary to that root so historical reads no longer require an archived whole generation.
2. Only after (1), exclude root-backed anchors from `pinned_historical_generations`; then single-file/directory compaction can physically stop carrying the old generation and atom reclamation becomes the sole physical retention law.
3. Add store-level crash/reopen hostile for a real migration boundary whose source root is captured into CFPR v2, survives both backends, and is released under torn-publication matrices.
4. Keep durable non-direct expressions materialize-before-publication unless a serialized expression law is proved.

### OPEN — DEFERRED / MANDATORY CARRY
- Context reference/optional-reference patches, relationship mutations, safe create/delete, final Database/Context DX;
- field-granular certified changes;
- DB-owned granular authorization for query/change/history/watch/hosted sessions;
- deterministic regex/Matches, richer Semantic Rules, entity/model invariants, transaction `require`;
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics;
- final Python/.NET/Studio, backup/restore/corruption UX, public perf/binary budgets, native Windows secure-memory expansion.

### SUPERSEDED / REJECTED
- REJECTED: one independently encoded CFPR/atom store per historical revision.
- REJECTED: copying an old-only atom under a fresh ID merely to avoid an immutable atom-ID collision.
- REJECTED: dropping generation-level history pins before historical semantic registry/recovery authority can be reconstructed from the retained root.
- Carry prior rejects: second history store, migration-progress journal, eager logical reconstruction as physical reopen, SQL fallback, source-A current routing, runtime witness/cache serialization.

### NEXT RECOMMENDED PASS
**PASS433:** close `HistoricalEpochMaterial` over retained realization roots: durable semantic-context/registry binding + exact historical causal boundary, then make generation pinning conditional on the absence of root-backed authority. Prove compaction physically discards the old generation while historical reads still reopen from shared atoms; release must then reclaim old-only atoms and leave shared atoms live.

## PASS433 — complete root-backed historical revision authority

### CLOSED THIS PASS
- CFPR is now v3. A retained historical realization root may carry its exact historical `SemanticContext` alongside causal `RevisionId` and direct root topology; physical atom payloads remain shared once in the same `PhysicalAtomStore`.
- Historical semantic context is versioned with the checkpoint context codec. CFPR v1/v2 remain decodable; v2 historical roots decode as context-incomplete and therefore conservatively keep their old generation pin.
- `DurableFactorizedRealization::historical_revision` materializes the historical logical `DatabaseState` from the retained root only when requested, then validates/builds the exact `Revision` against the shared durable `SemanticRegistry`.
- `DurableRevisionStore::historical_revision_from_realization` additionally binds the reconstructed revision to the exact `HistoricalEpochAnchor` source revision/schema; mismatch fails closed.
- Runtime `revision_at()` now prefers complete root-backed authority at a schema boundary and falls back to legacy `HistoricalEpochMaterial` checkpoint/WAL replay only when no complete root exists. There is still one history engine and one causal effect ledger.
- `pinned_historical_generations` excludes only anchors whose retained root has matching source revision, persisted historical semantic context, matching source schema, and a registry-valid context. Legacy/incomplete roots remain pinned.
- Directory hostile proves an old generation checkpoint/manifest is physically removed by compaction while exact source `Revision` remains reconstructible before and after reopen; release still works after that generation is gone.
- Single-file hostile found and closed a leftover duplicate authority: outgoing historical archive creation is now conditional on the *post-capture* pin set computed against the candidate physical realization. Complete root-backed history no longer creates/carries a whole-generation archive.

### SELECTED LAW
```text
HistoricalRevisionRoot(effect) =
    exact source RevisionId
  + exact historical SemanticContext
  + direct historical RealizationRoot
  + shared durable SemanticRegistry
  + shared PhysicalAtoms reachable by that root

revision_at(source):
    if complete HistoricalRevisionRoot exists:
        state := evaluate(root, shared atoms)
        Revision::build(source_id, historical_context, durable_registry, state)
    else:
        legacy HistoricalEpochMaterial checkpoint/WAL authority

pin(old_generation)
    iff retained anchor lacks complete root-backed authority
```

### OPEN — IMMEDIATE
1. Extend root-backed history hostile through streaming/torn-publication matrices so a context-complete historical root is never considered authoritative before the containing CFPR/root publication.
2. Audit historical root scaling: many retained migration boundaries must share semantic modules/contexts and atom payloads without O(history * schema) accidental duplication; factor context authority if measurements justify it.
3. Audit historical `revision_at()` cost and add bounded/lazy coordinate reads over historical realization roots before exposing large-history product UX; full logical materialization is now correctness fallback at the read boundary, not storage authority.
4. Continue durable non-direct realization work only when a serialized expression law is proved; current durable authority remains direct/materialized and fail-closed otherwise.

### OPEN — DEFERRED / MANDATORY CARRY
- Context reference/optional-reference patches, relationship mutations, safe create/delete, final Database/Context DX;
- field-granular certified changes;
- DB-owned granular authorization for query/change/history/watch/hosted sessions;
- deterministic regex/Matches, richer Semantic Rules, entity/model invariants, transaction `require`;
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics;
- final Python/.NET/Studio, backup/restore/corruption UX, public perf/binary budgets, native Windows secure-memory expansion.

### SUPERSEDED / REJECTED
- SUPERSEDED for complete roots: whole-generation pin/archive as the primary historical physical authority.
- REJECTED: persisting a WAL prefix inside each `HistoricalRevisionRoot`; the root already denotes the exact source causal boundary by `RevisionId`.
- REJECTED: treating v1/v2/context-less roots as complete and dropping their generation pin.
- Carry prior rejects: second history store, inverse-current-schema migration routing, atom-ID remapping, migration-progress journal, SQL fallback, runtime witness/cache serialization.

### NEXT RECOMMENDED PASS
**PASS434:** crash/streaming hostile for complete historical-root publication + retained-history scaling/cost audit. If clean, begin replacing full historical `Revision` materialization on `revision_at()` with root-backed lazy/read-context authority while preserving the exact same semantic API.

## PASS434 — streaming crash closure + retained-root scaling + lazy historical read substrate

### CLOSED THIS PASS
- Interrupted streaming publication of a context-complete historical root is proven non-authoritative before final publication: reopen stays on the previous checkpoint physical root, recovers the target through WAL, and continues to use the pinned old generation for historical source authority.
- Retrying and successfully finalizing the same streaming checkpoint captures the complete historical root, removes the old-generation pin, permits compaction to delete the old checkpoint/manifest, and preserves exact historical reconstruction after reopen.
- Historical-root scaling hostile: current + 64 retained roots represent 195 naive per-root atom slots but only 67 unique atoms in one shared graph; CFPR roundtrip preserves all 64 retained roots.
- `kernel-realization::evaluate_relation_expr_factorized` is now a public universal primitive for executing the complete current `RelExpr` IR directly over a factorized realization without materializing full `DatabaseState`.
- Lazy historical-read hostile rejects cloning `PhysicalAtomStore` into every `ReadContext`: the current store is a deep-cloned `BTreeMap`, which would reintroduce O(all atoms) memory per historical handle.

### SELECTED IMPLEMENTATION / R&D LAW
```text
Historical query semantics
    = same RelExpr + SemanticContext + SemanticRegistry
    executed against either logical Revision authority
    or factorized RealizationRoot authority

NO second query engine
NO historical-specific semantic API

Before long-lived realization-backed ReadContext:
    PhysicalAtomStore ownership must become cheap immutable sharing
    (Arc/persistent authority), not deep clone.
```

### OPEN — IMMEDIATE
1. Make immutable PhysicalAtom authority cheaply shareable across current root, retained historical roots, and read handles; measure Arc/COW vs kernel-persistent path-copy ownership and choose one law.
2. Refactor runtime `ReadContext` around one representation-neutral read authority and use factorized `RelExpr` execution for historical reads without full `Revision` materialization.
3. Add direct factorized point/entity/field read lowering under the same read authority; do not invent per-operation historical APIs.
4. Extend torn-publication hostile to the remaining single-file root-switch boundaries if the read-authority refactor touches those paths.

### OPEN — DEFERRED / MANDATORY CARRY
- Context reference/optional-reference patches, relationship mutations, safe create/delete, final Database/Context DX;
- field-granular certified changes;
- DB-owned granular authorization for query/change/history/watch/hosted sessions;
- deterministic regex/Matches, richer Semantic Rules, entity/model invariants, transaction `require`;
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics;
- final Python/.NET/Studio, backup/restore/corruption UX, public perf/binary budgets, native Windows secure-memory expansion.

### SUPERSEDED / REJECTED
- REJECTED: cloning the complete `PhysicalAtomStore` into each historical `ReadContext`.
- REJECTED: parallel `historical_query()` / `historical_read_field()` semantic surface independent from ordinary `ReadContext`.
- Carry prior rejects: second history/atom store, generation retention for complete roots, migration-progress journal, SQL fallback, eager physical reopen through logical reconstruction.

### NEXT RECOMMENDED PASS
**PASS435:** immutable/shareable PhysicalAtom authority + unified realization-backed `ReadContext` foundation. Preserve one semantic/query law and make read-handle/root cloning proportional to root metadata/path changes rather than total atom payload.

## PASS435 — shared atom authority + factorized historical ReadContext

### CLOSED THIS PASS
- `PhysicalAtomStore` moved from owned `BTreeMap<PhysicalAtomId, PhysicalAtom>` to `PersistentOrdMap<PhysicalAtomId, Arc<PhysicalAtom>>`: snapshot clones share topology/payload allocations; sparse writes path-copy only persistent nodes.
- Historical realization merge no longer rebuilds/clones the entire atom map/payload union. `merge_exact_from` preserves exact `PhysicalAtomId` identity and Arc allocation when payloads agree; conflicts fail closed.
- Added weak structural/payload probes proving path-copied nodes and branch-only atom allocations reclaim after the branch drops.
- `Database::at(revision)` now prefers an exact complete retained factorized root when one exists. `ReadContext` carries one representation-neutral read authority and ordinary `PreparedQuery` executes through `evaluate_relation_expr_factorized` without full historical `Revision/DatabaseState` materialization.
- Arc whole-map COW rejected as the main authority model: it copies O(N) map topology on first mutation while pinned. Persistent ownership preserves O(k log N) structural growth for sparse root rewrites.

### PERFORMANCE BASELINE
- N=100,000, pinned snapshot, update count 1: persistent 4.436 us vs whole-map Arc COW 743.420 us.
- update count 16: 12.369 us vs 466.343 us.
- update count 256: 159.690 us vs 573.254 us.
- update count 1024: 1019.045 us vs 612.833 us. This crossover is accepted: large one-owner batches may favor contiguous BTreeMap mutation, while versioned retention law remains persistent.

### OPEN — IMMEDIATE
- Extend the representation-neutral read authority to direct entity/field/point reads, not only relational `PreparedQuery`, without adding historical-specific APIs.
- Hostile-test exact historical revisions that are not themselves retained migration-source roots; avoid eager logical materialization where an exact realization/delta authority can serve them.
- Audit atom-store bulk construction/materialization paths so large one-owner builds can use sorted bulk construction rather than repeated persistent inserts.
- Keep non-direct durable realization fail-closed until serialized expression semantics are proved.

### OPEN — DEFERRED / MANDATORY CARRY
- Context reference/optional-reference patches, relationship mutations, safe create/delete, final Database/Context DX;
- field-granular certified changes;
- DB-owned granular authorization for query/change/history/watch/hosted sessions;
- deterministic regex/Matches, richer Semantic Rules, entity/model invariants, transaction require;
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics;
- final Python/.NET/Studio surfaces, backup/restore/corruption UX, public perf/binary budgets, native Windows secure-memory expansion.

### SUPERSEDED / REJECTED
- REJECTED: deep-cloning `PhysicalAtomStore` for each historical read handle.
- REJECTED: whole-map Arc COW as the versioned physical atom authority.
- REJECTED: separate historical query APIs/engine.

### NEXT RECOMMENDED PASS
PASS436: finish representation-neutral read authority for direct entity/field/point access and hostile historical revisions between retained physical roots; then audit large bulk atom construction versus persistent sparse mutation.

## PASS436 — factorized intermediate historical revisions

### CLOSED THIS PASS
- Added an exact relation-only historical derivation law: a retained complete factorized root can advance through reversible `RelationData` / `RelationRewrite` / `RelationResolution` effects without constructing the surrounding historical `DatabaseState`.
- `FactorizedRealizationRoot::apply_exact_relation_delta_endpoint` materializes only the touched relation, applies the authoritative `kernel-query::RelationDelta`, and installs a fresh direct columnar endpoint. Untouched atom payloads/map topology remain structurally shared.
- `DurableFactorizedReadSnapshot::advance_relation_delta` preserves the same historical `SemanticContext` and persistent atom authority while moving to the exact target `RevisionId`.
- `DurableRuntime::factorized_read_snapshot_at` now searches forward from retained complete historical roots across exact relation-only effects. Unsupported/mixed/schema/model transitions are not approximated; ordinary `revision_at()` remains the exact authority for those cases.
- Direct object/ref/point product reads were audited: `ObjectSet::get`, `Ref::load`, relationship loads and projections already lower through the ordinary `PreparedQuery`/`ReadContext::execute` surface. Therefore they automatically consume factorized read authority and no historical-specific/direct duplicate API was added.

### SELECTED IMPLEMENTATION / R&D LAW
```text
retained factorized root at R0
    + exact reversible relation effect R0 -> R1
        -> decode/materialize ONLY touched relation
        -> kernel-query exact RelationDelta
        -> fresh direct factorized relation endpoint
        -> R1 factorized read snapshot

untouched physical atoms: shared
full historical DatabaseState: not constructed

mixed model/schema effect without proved factorized lowering:
    no approximation / no SQL fallback
    -> existing exact logical revision_at() path
```

### OPEN — IMMEDIATE
1. Extend factorized historical derivation to exact `DurableModelDelta` coordinates (carrier/field/lifecycle/keeps-alive) without full-world reconstruction; derive one universal model-delta-to-realization law rather than per-field routing.
2. Add retained-root path selection/caching only if history-depth measurements justify it; do not persist derived intermediate snapshots as a second history store.
3. Audit touched-relation cost: current relation-only derivation intentionally materializes one affected relation. R&D a witness/column delta application path if large-relation history shows this is a real bottleneck.
4. Defer transient/wide persistent-tree bulk optimization from P435 until correctness/product blockers below are closed.

### OPEN — DEFERRED / MANDATORY CARRY
- Context reference/optional-reference patches, relationship mutations, safe create/delete, final Database/Context DX;
- field-granular certified changes;
- DB-owned granular authorization for query/change/history/watch/hosted sessions;
- deterministic regex/Matches, richer Semantic Rules, entity/model invariants, transaction `require`;
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics;
- final Python/.NET/Studio surfaces, backup/restore/corruption UX, public perf/binary budgets, native Windows secure-memory expansion.

### SUPERSEDED / REJECTED
- REJECTED: eager full-world `DatabaseState` reconstruction for relation-only intermediate historical revisions when a complete retained realization root exists.
- REJECTED: separate historical point/entity APIs; ordinary object reads already lower through the shared query/read-authority surface.
- REJECTED: guessing factorized semantics for mixed model/schema transitions before an exact lowering law exists.

### NEXT RECOMMENDED PASS
**PASS437:** derive exact `DurableModelDelta -> FactorizedRealizationRoot` history lowering for carriers/fields/lifecycle/keeps-alive, then hostile mixed relation+model intermediate revisions. If that closes cleanly, return to the long-deferred Context field/reference/relationship + field-granular change-coordinate product line before bulk persistent-tree tuning.

## PASS437 — exact factorized DurableModelDelta closure

### CLOSED THIS PASS
- Added one exact `DurableModelDelta -> FactorizedRealizationRoot` action in `DurableFactorizedReadSnapshot::advance_model_delta` covering carrier presence/membership, field endpoints, lifecycle entities/roots and keeps-alive edges.
- The lowering rewrites only touched direct physical authorities, validates the resulting direct topology, and prunes to exact root dependencies; it never constructs the surrounding `DatabaseState`.
- `DurableRuntime::factorized_read_snapshot_at` now advances retained factorized history across exact `MixedRevision` effects as well as relation-only effects. Relation and model deltas compose at one target `RevisionId`; schema/semantic-change/full boundaries remain excluded.
- Hostile model-only and mixed relation+model fixtures are extensionally equal to the exact logical target.

### SELECTED LAW
```text
retained factorized R0
 + exact reversible effect R0 -> R1
      relation mutations -> touched relation direct endpoints
      DurableModelDelta  -> touched lifecycle/carrier/field endpoints
 = factorized R1

untouched atom topology/payloads remain shared
full historical DatabaseState is not constructed
```

### OPEN — IMMEDIATE
1. Stop extending storage/history unless new hostile evidence appears; ordinary reversible relation+model intermediate history is now closed under factorized read authority.
2. Return to Context reference/optional-reference patches and relationship mutation calculus.
3. Introduce field-granular certified change coordinates so independent field writes do not conflict through one physical relation row.
4. Then enforce granular DB-owned authorization on those semantic coordinates.
5. Resume deterministic/general Semantic Rules, entity/model invariants and transaction `require` on the common expression substrate.

### OPEN — DEFERRED / MANDATORY CARRY
- transient/wide persistent-tree + sorted bulk builder R&D from P435;
- safe create/delete and final Database/Context DX;
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics;
- final Python/.NET/Studio surfaces, backup/restore/corruption UX, public perf/binary budgets, native Windows secure-memory expansion.

### SUPERSEDED / REJECTED
- SUPERSEDED for exact ordinary reversible mixed history: eager full-world historical `DatabaseState` reconstruction when a complete retained factorized root is reachable.
- REJECTED: per-feature historical model routing; carrier/field/lifecycle/keeps-alive use one model-delta action.
- Carry prior rejects: second history/query store, SQL fallback, historical-specific read APIs, physical atom ID remapping.

### NEXT RECOMMENDED PASS
**PASS438:** return to the deferred product line: Context reference/optional-reference patch calculus + relationship mutations, then field-granular change coordinates. Storage/history architecture should move again only on concrete hostile evidence.

## PASS438 — Context field/reference patches + durable field coordinates
CLOSED THIS PASS: partial Context scalar/reference/optional-reference patching; partial relationship attach/detach preservation; durable `DurableObjectFieldWrite`; exact `RuntimeHistoryCoordinate::ObjectField`; stale independent-field semantic reapply; same-field conflict; stable retry identity via mixed residual dual authority; legacy v2-v10 relation-mutation decoding after v11 field metadata.
OPEN — IMMEDIATE: DB-owned granular authorization over these semantic coordinates.
OPEN — DEFERRED: deterministic Semantic Rules/invariants/transaction require; migration frontend/product bindings; transient/wide persistent-tree batch R&D remains performance-only.
NEXT RECOMMENDED PASS: PASS439 authorization substrate over query/change/history/watch/session semantic coordinates.

## PASS439 — semantic authorization footprint + granular read/field-write authority

### CLOSED THIS PASS
- Added one storage/frontend-neutral `kernel-query::RelReadFootprint` calculus over `RelExpr`. Authorization dependencies are derived from query semantics rather than from client type shape, SQL-style table fallback, or blanket scan-column routing.
- `Project` propagates only observed source columns; filters/order/top-k add their controlling column; joins add join keys plus observed outputs; anti-join adds only the right key dependency; Group adds grouping/value dependencies; Difference/Union/Distinct conservatively include the full row-equality dependency because hidden columns can affect observable membership/deduplication.
- `PreparedQuery`, ordinary `ReadContext`, Candidate execution and Watch creation all enforce the same footprint through shared `RuntimeAuthority`; historical factorized reads inherit the same gate because they execute the same prepared query authority.
- Added granular session grants `ReadRelation`, `ReadField`, `WriteRelation`, `WriteField` while preserving coarse `Read`/`Write` as explicit whole-database grants for compatibility.
- P438 object field-write intent now meets read authorization at one stable semantic coordinate: object relation column IDs are derived from the existing `cfmd.object.kernel-field.v1` field identity instead of ordinal `1..N`. Relationship carrier columns also receive stable semantic IDs rather than ordinal identity.
- `commit_bound_plan` rechecks current shared session authority at publication: raw relation mutations require relation-write authority; semantic object field patches require exact field-write authority. Refreshed/revoked sessions therefore cannot retain stale grants inside old plans.
- Hostile regression proves one-field read grants admit `Scan -> Project(field)` but reject a full-row scan, and one-field write grants admit the matching P438 field patch while rejecting a raw whole-relation insert.

### SELECTED IMPLEMENTATION / R&D LAW
```text
Authorize(observation Q)
    = authorize( semantic_read_footprint(Q) )

semantic_read_footprint
    : RelExpr -> finite set of
        Relation(r)
      + Field(r, stable_column_id)

Authorize(change C)
    = authorize( exact semantic write coordinates carried by C )

Context shape != security
frontend syntax != security
physical realization != security coordinate
```

The read-footprint is an information-dependency law, not merely a list of projected output columns: any hidden coordinate that can change row survival, ordering, grouping, equality/deduplication, or join membership is part of the authorization footprint.

### HOSTILE / REJECTED
- REJECTED: authorizing only `scan_relations()`; it cannot express field secrecy.
- REJECTED: authorizing every column of every scanned relation as the universal implementation; it destroys legitimate field-granular projection and is not the semantic dependency law.
- REJECTED: Context omission as a grant/deny mechanism.
- REJECTED: host/Python/Rust-specific ACL engines. Host authorizers may establish a principal/grant set, but database/runtime execution owns enforcement.
- REJECTED: ordinal object-column identity for authorization; rename/reorder/migration-safe policy must bind stable semantic coordinates.

### PERFORMANCE BASELINES TO PRESERVE
- Footprint derivation is structural in query IR / referenced coordinates, not database cardinality; it performs no row scan and allocates only finite ordered coordinate sets.
- Authorization checking scales with footprint/grant size, not relation cardinality or physical atom count.
- Preserve P397-P400 native-cost realization baselines and P435 persistent-sharing baselines; P439 must remain above physical execution and must not reintroduce per-row authorization routing.

### OPEN — IMMEDIATE
1. Classify change actions exactly into field write, generic relation mutation, relationship attach/detach/move, object create and object delete. Preserve existing stronger relationship/lifecycle semantics; do not collapse them into a generic `WriteRelation` grant merely for convenience.
2. Add explicit create/delete/relationship permission vocabulary only together with the exact classifier/enforcement law; do not expose dead grants before enforcement exists.
3. Close history/undo/redo authorization against the coordinates of the historical effect being materialized, while retaining separate `HistoryRead` metadata authority.
4. Prove Watch uses both `Watch` lifecycle authority and the same exact read footprint under authorization refresh/revocation, including hosted sessions.
5. Audit write-only DX: a principal allowed to mutate one field should not need read access to unrelated hidden fields merely because patch formation internally needs the authoritative row. Internal DB reads must not become externally observable read grants.

### OPEN — DEFERRED / MANDATORY CARRY
- safe create/delete and final Database/Context DX cleanup after the authorization action law;
- deterministic `Matches`/regex, richer Semantic Rules, entity/model invariants and transaction `require` on the common serialized expression substrate;
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics;
- final Python/.NET/Studio surfaces, backup/restore/corruption UX, public perf/binary budgets, native Windows secure-memory expansion;
- transient/wide persistent-tree sorted bulk builder R&D remains performance-only;
- storage/history/Physical Realization line remains closed absent new hostile evidence.

### SUPERSEDED / DO NOT EXTEND
- Context-shape-as-security;
- frontend-specific authorization engines;
- ordinal relation-column identity for object security policy;
- blanket all-columns authorization as a replacement for semantic query dependency analysis;
- SQL/table ACL fallback beneath the semantic query/change kernels.

### NEXT RECOMMENDED PASS
**PASS440 — exact change-action authorization closure:** derive create/delete/relationship/field/relation mutation authorization coordinates from existing object/lifecycle/change authority, then carry the same law through history undo/redo and hosted/watch refresh. Also remove the current write-only-DX read dependency without exposing hidden fields.

## PASS439 — semantic granular authorization substrate

### CLOSED
- One `kernel-query::RelReadFootprint` law authorizes exact semantic relation/field observations for ordinary, prepared, Candidate, historical-factorized and Watch query paths.
- Stable semantic object-field IDs, not relation ordinals/source spelling, are the authorization coordinates.
- `ReadRelation` / `ReadField` / `WriteRelation` / `WriteField` are DB/runtime-owned grants; Context shape and frontend-specific ACL engines remain rejected.

### CARRY
- Exact object lifecycle and relationship action grants.
- History inverse/redo action authorization.
- Hosted/watch grant-refresh hostile.
- Write-only mutation formation must not require unrelated observable reads.

## PASS440 — exact object/relationship change-action authorization

### CLOSED THIS PASS
- Added semantic write grants `CreateObject`, `DeleteObject`, `AttachRelationship`, `DetachRelationship`, `MoveRelationship` while preserving coarse `Write` / `WriteRelation` as explicit supersets.
- High-level object/relationship APIs stamp exact mutation-action authority into `Plan` at construction. Raw `Plan::insert/remove` remains generic relation-write and cannot acquire object/relationship authority by sharing the same physical relation.
- Action stamps are keyed by `(relation, insert/remove, mutation ordinal)`; authorization is O(number of planned mutations) with no row matching/search and no O(N^2) classifier.
- Stale/rebased publication authorizes and rechecks the original semantic action intent; residual rows are internal realization and do not collapse back to generic relation-write authority.
- `OwnedMany(orphan = delete)` detach requires both exact relationship-detach authority and target `DeleteObject` authority. Induced lifecycle deletion is therefore not an authorization bypass. Atomic move does not acquire false orphan-delete authority.
- Write-only field mutation is real: mutation planning has a crate-private non-observable read path. `WriteField` can form/commit the semantic patch without `Read`/`ReadField`; public snapshot/query execution remains read-authorized and fails closed.
- Session object handles/transaction formation no longer require an observable read grant merely to construct a write intent. Actual query/materialization remains governed by the P439 footprint law.

### HOSTILE / REJECTED
- REJECTED: infer create/delete/attach/detach/move from final relation deltas at commit time. Physical row shape is not the semantic action and becomes ambiguous under rebase/normalization.
- REJECTED: let any semantic action grant authorize every mutation on that relation. Raw relation writes remain separate.
- REJECTED: orphan cascade under `DetachRelationship` alone; induced target deletion needs target delete authority.
- REJECTED: grant hidden read access merely because mutation formation needs authoritative state internally.

### VERIFICATION
- `cfmd-runtime --tests`: 48 passed / 0 failed.
- `cfmd` public surface: 49 passed / 0 failed.
- `cfmd` public API contract: 1 passed / 0 failed.
- `cfmd-host`: 8 passed / 0 failed.
- Hostile coverage includes write-only field patch, create/delete vs raw relation write, relationship move vs attach, and owned-detach induced-delete authority.

### OPEN — IMMEDIATE
1. History undo/redo must transport/recover the exact semantic action coordinates instead of rebuilding a raw relation-write Plan from historical row deltas.
2. Hosted/watch grant refresh: prove field/relation footprint and lifecycle `Watch` authority are both rechecked under refresh/revocation without a second ACL engine.
3. Audit model-only/explicit migration Plan paths so restricted sessions cannot obtain a semantic write class not represented by the action calculus; migration/admin authority should remain explicit rather than accidental `Write` leakage.
4. Decide whether product-level named roles are merely immutable/composable `PermissionSet` presets or require a durable DB-owned role object. Do not add a second authorization semantics layer.

### OPEN — R&D / CONTEXT DX DESIGN GOAL (SYNTAX DELIBERATELY UNDECIDED)
Zero-downtime remote-reader compatibility across schema-semantic migration remains a required DX design problem.

Constraints agreed so far:
- a reader service may contain no authoritative schema definition at all;
- the DB/server must not need reader-specific contract annotations/markers;
- no extra reader-contract handshake should be required solely for this feature; current schema version/epoch belongs in normal DB metadata already needed by the remote read authority;
- reader-side local field compatibility may depend deterministically on that schema version/epoch;
- the same persisted/source field spelling may denote a different semantic field after migration, so name/existence fallback is invalid;
- resolution should happen once when binding the Context/ReadContext for a schema epoch, never as per-query/per-row routing;
- incompatible/uncovered schema epochs fail closed;
- authoritative schema stays current-only and legacy-free; compatibility history is consumer-side;
- exact public annotation/API syntax is NOT selected yet. In particular `bind/rebind` is only a discarded/unfinished naming discussion, not an accepted DX.

Design target: the simplest client-side compatibility calculus that can prepare a remote reader before an A -> B migration, survive the cutover without a restart race, and distinguish same-name/different-semantics fields without server-side reader configuration.

### OPEN — DEFERRED / MANDATORY CARRY
- deterministic `Matches`/regex and richer serialized Semantic Rules;
- entity/model invariants and transaction `require` on the common semantic-expression substrate;
- final Database/Context creation/open DX cleanup after the reader-compatibility design above is resolved;
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics;
- final Python/.NET/Studio, backup/restore/corruption UX, public perf/binary budgets, native Windows secure-memory expansion;
- transient/wide persistent-tree + sorted bulk-builder R&D remains performance-only.

### NEXT RECOMMENDED PASS
**PASS441 — history/watch authorization closure:** preserve semantic object/relationship/field action authority through historical undo/redo, then hostile hosted/watch refresh/revocation on the same shared authorization law. Keep reader migration-compatibility as an explicit DX/R&D ledger item but do not choose its syntax until the user returns to it.


## PASS441 — history/watch authorization continuation
- Exact semantic authorization footprint is carried durably into history; inverse authority maps Create↔Delete, Attach↔Detach, Move→Move, field→same field.
- History inverse coverage survives transaction composition and is re-persisted, so undo-of-undo does not degrade to generic WriteRelation.
- Watch authorization retains the exact query read footprint and revalidates on use/host authorization refresh; field/relation read revocation fails closed even when Watch remains granted.
- OPEN R&D: schema-version-aware remote-reader compatibility for semantic field changes. Constraints: reader-side; no server reader contracts/metadata, no dedicated handshake, no per-query routing/name-existence fallback; resolve deterministically from ordinary schema metadata at Context/ReadContext boundary; authoritative schema stays current-only; final DX syntax intentionally undecided.
- OPEN NEXT: named Role DX + explicit administrative/model/migration authority on the same DB-owned permission law.

## PASS442 — role composition + administrative/model authority

### CLOSED THIS PASS
- Added named immutable `Role` bundles for application/authorizer DX. Roles flatten into the existing `PermissionSet`; role names never enter runtime authorization, there is no hierarchy, no role cache, and no second enforcement engine.
- Added `Permission::ModelRead`. Full `ReadContext::schema()` is now fallible and requires explicit model authority. Data `Read`, `ReadRelation`, and `ReadField` do not expose the complete authoritative model.
- Added `ReadContext::schema_revision()` as narrow schema-epoch metadata. This is intentionally sufficient groundwork for the open remote-reader migration DX without granting full model introspection.
- Added `Permission::SchemaMigrate` and restricted `SessionDatabase::migrate`. Generic `Write` does not authorize schema migration.
- Migration performs cheap authority admission before preparation and linearizes current `SchemaMigrate` authority over the durable publication call; expensive migration preparation does not hold the shared session lock.
- Fixed hosted terminal-error semantics discovered by full hostile verification: a revoked session maps to `SessionClosed`, and a blocked watch rechecks current authorization when cancellation wakes it, so permission-loss vs ordinary watch cancellation is deterministic.
- Removed an obsolete protocol-test assumption that every output-equivalent causal revision emits an empty watch event. Current watch law quotients empty revisions; the next observable event spans the exact source/target revision interval.

### SELECTED LAW
```text
Role(name, grants...) --flatten--> PermissionSet
                                  |
                                  +--> one RuntimeAuthority law

DataRead != ModelRead
DataWrite != SchemaMigrate

migration prepare
    -> revalidate/hold SchemaMigrate at publication
    -> durable schema publication
```

### HOSTILE / REJECTED
- REJECTED: durable/runtime role hierarchy or role-name checks. That duplicates permission semantics and makes refresh/revocation ambiguous.
- REJECTED: `Read` implicitly exposing the whole authoritative schema; a one-field reader must not discover hidden model coordinates.
- REJECTED: generic `Write` authorizing migration because both eventually modify persisted state.
- REJECTED: holding a session authority lock during migration compilation/materialization preparation; only publication needs a linearized authority boundary.
- REJECTED: changing current watch semantics back to empty-event emission merely to satisfy an obsolete protocol test.

### OPEN — IMMEDIATE
1. Audit any remaining administrative/runtime maintenance surfaces that may later become remotely reachable (backup/restore/key-management/retention/compaction) and assign explicit authority only when they enter `SessionDatabase`/hosted product surface; do not create speculative dead permissions.
2. Generalize P441 history authorization coverage composition from fail-closed same-relation precomposition only if real transaction composition requires it.
3. Return to deterministic Semantic Rules / entity-model invariants / transaction `require` after authorization closure is stable.

### OPEN — R&D / CONTEXT DX DESIGN GOAL (SYNTAX DELIBERATELY UNDECIDED)
- Remote reader may have no authoritative schema code.
- Server carries no reader-specific contracts/annotations and performs no dedicated compatibility handshake.
- Normal read metadata exposes schema epoch/version without requiring `ModelRead`.
- Reader compatibility is selected deterministically once per Context/ReadContext schema epoch; never per query/row and never by field-existence/name fallback.
- Same spelling may represent different semantics across schema epochs; uncovered epochs fail closed.
- Authoritative schema remains current-only; compatibility history remains consumer-side.
- Final annotation/API syntax remains intentionally undecided; `bind/rebind` is not accepted terminology.

### OPEN — DEFERRED / MANDATORY CARRY
- deterministic `Matches`/regex and richer serialized Semantic Rules;
- entity/model invariants and transaction `require`;
- final Context/Database creation/open DX cleanup after remote-reader compatibility design;
- migration frontend Rust/Python/TMD/CLI + diagnostics;
- final Python/.NET/Studio, backup/restore/corruption UX, public perf/binary budgets, native Windows secure-memory expansion;
- transient/wide persistent-tree sorted bulk-builder R&D remains performance-only.

### SUPERSEDED / DO NOT EXTEND
- Context shape as security;
- frontend/host-specific ACL engines;
- role-name/hierarchy enforcement separate from `PermissionSet`;
- generic `Write` as administrative/schema authority;
- full model disclosure as a side effect of ordinary data reads;
- per-query reader schema-version routing.

### PERFORMANCE BASELINES TO PRESERVE
- Role composition is O(total grants in selected roles), performed at authority construction/refresh, not query execution.
- Runtime enforcement remains structural in semantic query/change footprint, independent of row count/physical atom count.
- Migration authority adds no work during migration transforms; the shared session read lock is held only across the final durable publication call.
- Preserve P397-P400 realization native-cost class and P435 persistent-sharing baselines.

### NEXT RECOMMENDED PASS
**PASS443 — authorization closeout + Semantic Rules transition:** hostile-audit remaining remotely reachable administrative surfaces without adding speculative permissions; if no live seam remains, freeze authorization architecture and move to deterministic `Matches` / common semantic-expression substrate for rules/invariants/transaction `require`.

## PASS443 — authorization freeze + common Semantic Rule predicate substrate

### CLOSED THIS PASS
- Hostile-audited the actually reachable hosted administrative surface. Hosted protocol exposes revision/query/history/commit/watch/session lifecycle only; backup/restore/key management/retention/compaction are not remotely reachable product operations today. No speculative dead permissions were added.
- Authorization architecture for the current product surface is now FROZEN through P443: exact query footprints (P439), semantic change actions/write-only mutation (P440), durable history/watch refresh (P441), Role composition + ModelRead/SchemaMigrate (P442). Future administrative permissions are introduced only together with a real reachable operation and the same RuntimeAuthority law.
- Introduced `kernel-schema::SemanticRuleExpr` as the common deterministic predicate substrate with boolean composition and typed scalar predicates (`I64Range`, `TextLength`, `TextOneOf`, `TextMatches`). Current field-rule constructors lower into this same expression law for schema type/bounds validation.
- Removed duplicated field-rule truth logic between state validation and dynamic violation/VMF measurement. Both now call one `kernel-validation::field_rule_matches` implementation, eliminating a semantic split where commit validity and violation mass could disagree.
- Added schema-owned regular-language `TextPattern` algebra: `Never`, `Empty`, `Literal`, `AnyScalar`, `Concat`, `Alternate`, `ZeroOrMore`. This is deterministic semantic data, not a Rust callback and not host-regex behavior.
- Added Thompson-style epsilon-NFA compilation and Unicode-scalar matching. Ambiguous regular patterns such as `(a|aa)*` do not backtrack exponentially.
- Hostile performance refinement removed `BTreeSet`/per-scalar closure allocation from the compiled matcher. `CompiledTextPattern` reuses vector frontiers and generation marks; matching after compile performs no per-character heap allocation.

### SELECTED LAW
```text
existing field sugar
    range / length / one_of
        | lower
        v
SemanticRuleExpr  <--- future Matches / invariants / require
        |
        | schema typecheck
        v
compiled deterministic rule plan
        |
        +--> authoritative validation
        +--> violation / VMF
```

`TextPattern` is a regular-language AST; textual regex syntax is deliberately not selected yet. Frontends must eventually compile into the same AST rather than delegate meaning to host regex engines.

### HOSTILE / REJECTED
- REJECTED: a second validation implementation for VMF/violation accounting. One predicate truth law now feeds both validity and violation measurement.
- REJECTED: Rust/Python/.NET regex callbacks or backend-specific regex libraries as semantic authority. Pattern meaning must be DB-owned and deterministic across bindings/recovery.
- REJECTED: recursive/backtracking matcher for `Matches`; ambiguous regular patterns must not create exponential work.
- REJECTED: compile the pattern automaton per row/scalar. The selected production boundary is serialized AST -> compiled reusable rule plan.
- REJECTED: invent backup/key/compaction permissions before those operations exist on restricted/hosted surfaces.

### OPEN — IMMEDIATE
1. Persist/expose the common `SemanticRuleExpr` through schema/runtime durability instead of keeping `Matches` kernel-only; compile/cache rule plans once per semantic context/schema validation boundary, never once per value.
2. Add the minimal frontend `Matches` DX only after its textual syntax is chosen; the semantic target is already `TextPattern`, not a host regex engine.
3. Extend `RuleValueExpr` from field-input-only to stable semantic field coordinates and use the same predicate substrate for entity/model invariants.
4. Add transaction `require` only after candidate/future-world expression binding is exact; it must reuse the same predicate truth/typecheck law, not become an application callback/precondition engine.

### OPEN — R&D / REMOTE-READER SCHEMA-EVOLUTION DX
- Reader may have no authoritative schema code.
- Server carries no reader-specific contract configuration and no dedicated compatibility handshake.
- Normal metadata exposes schema epoch/version without `ModelRead`.
- Compatibility resolves once per Context/ReadContext schema epoch, never per query/row and never by field-name/existence fallback.
- Same spelling may have different semantics across epochs; uncovered epochs fail closed.
- Authoritative schema remains current-only and compatibility history consumer-side.
- Public annotation/API syntax remains deliberately undecided; `bind/rebind` is not selected.

### OPEN — DEFERRED / MANDATORY CARRY
- final Context/Database creation/open DX cleanup after remote-reader compatibility design;
- migration frontend Rust/Python/TMD/CLI + diagnostics;
- final Python/.NET/Studio surfaces;
- backup/restore/corruption UX and exact admin permissions when those surfaces become reachable;
- public performance/binary-size budgets and native Windows secure-memory expansion;
- transient/wide persistent-tree sorted bulk-builder R&D remains performance-only.

### SUPERSEDED / DO NOT EXTEND
- Context shape as security or frontend/host-specific ACL engines;
- role-name/hierarchy authorization separate from PermissionSet;
- speculative permissions for non-existent hosted operations;
- duplicated field-rule evaluators;
- host-language regex/callback validation semantics;
- exponential/backtracking pattern matching;
- per-value pattern compilation on validation hot paths.

### PERFORMANCE BASELINES TO PRESERVE
- Authorization remains structural in finite semantic footprints/grants and independent of row/physical-atom cardinality.
- Current range/length/one_of hot validation does not allocate a converted expression or clone membership sets per value.
- Compiled `TextPattern` matching uses reusable frontier buffers/generation marks and no per-character heap allocation; pattern compilation is a schema/rule-plan boundary operation.
- Preserve P397-P400 realization native-cost class and P435 persistent-sharing behavior.

### NEXT RECOMMENDED PASS
**PASS444 — durable/common Semantic Rules integration:** make `SemanticRuleExpr` an authoritative persisted schema rule, compile/carry reusable rule plans through validation/VMF, then expose deterministic `Matches` without choosing backend-specific regex semantics. After that extend the same expression coordinates toward entity/model invariants and transaction `require`.

## PASS444 — typed Context/Snapshot DX reduction + durable compiled Semantic Rules

### CLOSED THIS PASS
- Removed `ReadContext` from the root `cfmd` SDK surface. It remains runtime implementation vocabulary for query/object/watch internals and generated hidden glue, not an ordinary application concept.
- Established canonical public `Context<M>` as an alias over the existing typed database context. Ordinary reads stay on the moving current HEAD through `Context<M>`.
- Added typed `Snapshot<M>` returned by `Context::snapshot()` / `Context::at(revision)`. A snapshot binds the same typed entity surface to one exact immutable revision; no third `ReaderContext` abstraction exists.
- Preserved the previously selected transaction DX exactly: adaptive writes start with `Transaction::new()`, while the explicit strict-snapshot form is `Transaction::from(snapshot)`. `Snapshot<M>` implements the conversion without exposing runtime `ReadContext`; no `snapshot.transaction()` convenience path exists.
- Generalized `CfmdSchema`/`EntitySet` binding to an internal `ContextSource` (`Current` or exact `Snapshot`) so the same generated typed surface works for moving-current and fixed-revision reads without per-query routing.
- Hostile public test proves a live `Context<M>` sees a post-snapshot commit while the earlier `Snapshot<M>` does not; the existing strict snapshot transaction hostile now uses the typed snapshot DX.
- Promoted deterministic `Matches` from P443 kernel R&D into persisted authoritative schema semantics: `FieldRule::TextMatches(TextPattern)` now exists in kernel schema and public runtime schema vocabulary.
- Checkpoint durability encodes/decodes the complete regular-language `TextPattern` AST (`Never`, `Empty`, `Literal`, `AnyScalar`, `Concat`, `Alternate`, `ZeroOrMore`). Existing checkpoint roundtrip now contains a `TextMatches` rule and proves exact schema recovery.
- Added `CompiledFieldRule` + `CompiledRulePlan`. Full-state validation, relation-subset validation, and full dynamic violation/VMF measurement compile the schema rule plan once per validation/measure boundary and reuse the compiled Thompson NFA across values instead of compiling `Matches` per row.
- Existing field sugar (`range`, `length`, `one_of`) and persisted `Matches` share the same authoritative rule truth/type law; VMF continues to use the same semantics.

### SELECTED LAW
```text
Context<M>
    = typed consumer surface over moving current HEAD

Context<M>::snapshot()/at(R)
    -> Snapshot<M>
    = same typed surface + exact immutable revision R

ReadContext
    = runtime implementation authority only

persisted FieldRule
    -> deterministic SemanticRuleExpr / TextPattern
    -> CompiledRulePlan (once per validation semantic-context boundary)
    -> validation + VMF
```

### HOSTILE / REJECTED
- REJECTED: public `ReadContext` beside `Context<M>`; revision binding is a property of explicit `Snapshot<M>`, not a second application context family.
- REJECTED: `Snapshot::transaction()` as a second constructor spelling. Transaction construction remains owned by `Transaction`: `Transaction::new()` for adaptive intent and `Transaction::from(snapshot)` for strict temporal intent, as fixed in P355.
- REJECTED: making ordinary `Context<M>` snapshot-bound. Its entity sets intentionally reacquire current HEAD per operation; only explicit `snapshot()/at()` freezes revision.
- REJECTED: duplicating typed entity surfaces for snapshots. `ContextSource` binds the same generated `CfmdSchema`/`EntitySet` surface to current or exact-revision authority.
- REJECTED: `Matches` as host regex/callback or non-durable frontend sugar. The DB persists the regular-language AST itself.
- REJECTED: compiling the NFA once per row/value. Validation/VMF now have a compiled schema-rule plan boundary.
- The full parallel `cfmd-protocol` suite reproduced an existing P442 watch scheduling fixture flake (`NextWatch did not enter its consumer section`). The exact failing test passed immediately in isolation; no P444 product code touches that watch synchronization seam. Do not reinterpret this as a Semantic Rules/Context regression.

### VERIFICATION
- `kernel-schema`: 11 passed / 0 failed.
- `kernel-validation`: 19 passed / 1 ignored.
- `kernel-durability`: 248 passed / 2 ignored.
- `kernel-plan`: 297 passed / 5 ignored.
- `cfmd` public API contract: PASS.
- `cfmd` public surface before final typed-snapshot hostile: 49 passed / 0 failed; new typed snapshot hostile passes independently (suite is now 50 tests).
- `cfmd-host`: 8 passed / 0 failed.
- `cfmd-protocol` unit/main gates passed; hosted parallel suite exposed the carried scheduling flake above, while its exact failing test passes in isolation.
- Rustfmt is not present in the intentionally minimal Rust 1.98.1 toolchain; no component/archive was reinstalled merely for formatting.

### OPEN — IMMEDIATE
1. Promote the full boolean `SemanticRuleExpr` (not only `FieldRule` sugar variants) to durable schema authority, then extend `RuleValueExpr` from field-input-only to stable semantic field coordinates. P444 persisted `Matches` exactly, but general `And/Or/Not` expressions are still evaluator IR rather than first-class persisted rule records.
2. Introduce entity/model invariant rules over that exact deterministic expression/typechecking substrate and compile/cache invariant plans at the same semantic-context boundary; no callback validator or second VMF truth law.
3. Only after future-world binding is exact, add transaction `require` against Candidate semantics using the same expressions and coordinates.
4. Backend integration follow-up: restricted/session typed `Context<M>` should bind the same `ContextSource` through current `RuntimeAuthority`; do not create `ReaderContext`, `RoleContext`, or another typed surface. This is authorization plumbing, not a new DX concept.
5. Keep the P442 hosted watch scheduling fixture race visible; fix it only as a synchronization/test-boundary issue, not by weakening terminal authorization semantics.

### OPEN — R&D / REMOTE-READER SCHEMA-EVOLUTION DX
- Reader may contain no authoritative schema code.
- Server carries no reader-specific contract configuration and no dedicated compatibility handshake.
- Normal metadata exposes numeric schema epoch/version without `ModelRead`.
- Consumer-side compatibility resolves once when a `Context<M>` / `Snapshot<M>` binds to a schema epoch; no per-query/per-row routing and no field-existence/name fallback.
- Same spelling may have different semantics across epochs; uncovered epochs fail closed.
- Authoritative schema remains current-only; compatibility history remains consumer-side.
- Final annotation/API syntax remains deliberately undecided. `bind/rebind` is not selected terminology.

### OPEN — DEFERRED / MANDATORY CARRY
- final reader schema-evolution annotation/calculus;
- migration frontend Rust/Python/TMD/CLI + diagnostics;
- final Python/.NET/Studio surfaces;
- backup/restore/corruption UX and exact admin permissions when those surfaces become reachable;
- public performance/binary-size budgets and native Windows secure-memory expansion;
- transient/wide persistent-tree sorted bulk-builder R&D remains performance-only.

### SUPERSEDED / DO NOT EXTEND
- public `ReadContext` as ordinary application DX;
- any proposed `ReaderContext` type;
- Context shape as security;
- host-language regex/callback validation;
- per-value `TextPattern` compilation;
- independent rule evaluator for VMF;
- per-query schema-version routing.

### PERFORMANCE BASELINES TO PRESERVE
- Ordinary `Context<M>` reads have no schema/snapshot routing branch beyond selecting their operation source once.
- `Snapshot<M>` resolves the exact read source at bind time; individual entity operations do not look up revision history again.
- `CompiledRulePlan` construction scales with schema rule count, not data cardinality; NFA compilation is outside per-value validation loops.
- Preserve P397-P400 realization native-cost class and P435 persistent-sharing behavior.

### NEXT RECOMMENDED PASS
**PASS445 — persisted entity/model Semantic Rules:** first make full boolean `SemanticRuleExpr` a durable rule record, then extend it to stable semantic field coordinates, prove/typecheck entity/model invariants, compile them into the same rule-plan/VMF law, and prepare the exact Candidate binding required for transaction `require`. Keep session-bound `Context<M>` as a small backend authority-plumbing follow-up; do not introduce a new context type.

## PASS445 — persisted boolean Semantic Rules + stable entity invariant coordinates

### CLOSED THIS PASS
- Promoted the complete boolean `SemanticRuleExpr` into durable schema authority through `FieldRule::Expr`, including `True`, `False`, `And`, `Or`, `Not`, ranges, text length/membership and deterministic `TextMatches`.
- Extended `RuleValueExpr` with stable semantic `Field(SemanticId)` coordinates. Entity invariants therefore bind to semantic field identity, not Rust spelling, column ordinal or physical layout.
- Added schema-owned entity invariant records keyed by semantic owner type. Typechecking proves every referenced field exists, is available on the invariant owner through subtype closure, and has the scalar type required by the predicate.
- Extended `CompiledRulePlan` to compile field, relation-column and entity rules through one evaluator substrate. `TextPattern` is still compiled once to the non-backtracking NFA plan.
- Entity invariant truth participates in full finite-model validation and the same VMF/dynamic violation law through exact `EntityRule { owner, entity, rule_index }` witnesses.
- Runtime/public schema vocabulary now transports `SemanticRuleExpr` / `RuleValueExpr` and `SchemaBuilder::entity_rule(...)` into the kernel rule authority. No callback validator or frontend-owned evaluator was introduced.
- Checkpoint codec v4 persists both full boolean rule expressions and entity invariant records. Decode remains backward-compatible with checkpoint codec v1-v3.
- P444 Context/Snapshot/Transaction DX remains unchanged: adaptive `Transaction::new()`, strict `Transaction::from(snapshot)`; no `Snapshot::transaction()` and no public `ReadContext` regression.

### SELECTED LAW
```text
FieldRule sugar / FieldRule::Expr / entity invariant
                |
                v
        SemanticRuleExpr
                |
       RuleValueExpr::Input
       RuleValueExpr::Field(stable semantic field id)
                |
                v
        CompiledRulePlan
                |
        +-------+--------+
        |                |
 authoritative validation   exact VMF witnesses
```

Entity rules are model invariants in the exact finite-model sense that the rule is checked for every member of its semantic owner extent. Arbitrary model-wide relation quantifiers/aggregates are NOT claimed closed yet; they require a common relational/aggregate expression law rather than a second generic validator.

### HOSTILE / REJECTED
- REJECTED: infer invariant fields by source-language name or relation ordinal. Stable semantic field coordinates are the only accepted identity.
- REJECTED: add a separate entity-validator callback layer. Field rules and entity invariants use the same serialized expression, typecheck, compiled evaluation and VMF truth law.
- REJECTED: call arbitrary global predicates "model rules" before relational/aggregate coordinates have an exact common algebra. Such rules remain fail-closed/open rather than falling back to host callbacks or full-state generic scans hidden behind a second API.
- REJECTED: begin transaction `require` before proving the same expression binding against Candidate/future-world authority.

### VERIFICATION
- Workspace `cargo check --workspace --all-targets`: PASS.
- `kernel-schema`: new full-boolean/entity-coordinate hostile PASS.
- `kernel-validation`: **20 passed / 1 ignored**.
- `kernel-durability`: **248 passed / 2 ignored**; checkpoint roundtrip now contains both `FieldRule::Expr` and an entity invariant.
- `cfmd` public surface: **50 passed / 0 failed**.
- `kernel-plan` full suite compilation completed, but the monolithic run exceeded the external 45-second tool-call timeout during execution; targeted VMF publication gates `vmf_invariant_closure_certificate_is_zero_and_revision_bound` and `seal_rejects_nonzero_candidate_violation_state_before_publication` both PASS.

### OPEN — IMMEDIATE
1. Add an explicit hostile proving an entity invariant is evaluated against Candidate/future-world state through the same VMF authority, including a field change that is valid at source and invalid in the Candidate.
2. Once that future-world binding is proven, add transaction `require` as a passive semantic precondition on the same `SemanticRuleExpr`; no callback and no second precondition engine.
3. R&D the minimal exact algebra for genuinely model-wide invariants (relation membership/cardinality/aggregate/quantified constraints) by reusing query/aggregate semantic coordinates. Do not route these through host predicates or generic full-model fallback.
4. Decide final typed Rust DX for entity/model rules after the semantic algebra stabilizes; the current low-level `SchemaBuilder::entity_rule(TypeId, SemanticRuleExpr)` is substrate, not necessarily final ergonomic syntax.
5. Backend-only Context follow-up: restricted/session typed `Context<M>` should continue through the existing authority source; do not add `ReaderContext`/`RoleContext`.

### OPEN — R&D / REMOTE-READER SCHEMA-EVOLUTION DX
- Reader may have no authoritative schema code.
- Server carries no reader-specific contracts/annotations and performs no dedicated compatibility handshake.
- Schema epoch/version comes through ordinary metadata without `ModelRead`.
- Compatibility resolves once at `Context<M>` / `Snapshot<M>` binding, never per query/row and never by field-name/existence fallback.
- Same spelling may represent different semantics across epochs; uncovered epochs fail closed.
- Authoritative schema remains current-only; compatibility history remains consumer-side.
- Final annotation/API syntax remains undecided; `bind/rebind` is not selected terminology.

### OPEN — DEFERRED / MANDATORY CARRY
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics;
- final Python/.NET/Studio surfaces;
- backup/restore/corruption UX and exact admin permissions when those surfaces become reachable;
- public perf/binary-size budgets;
- native Windows secure-memory expansion;
- transient/wide persistent-tree sorted bulk-builder R&D remains performance-only.

### SUPERSEDED / DO NOT EXTEND
- public application `ReadContext` / any `ReaderContext`;
- `Snapshot::transaction()` or transaction construction outside `Transaction::new()` / `Transaction::from(snapshot)`;
- Context shape as security;
- host callback validators/regex semantics;
- independent VMF rule evaluator;
- source-language field-name/ordinal identity for invariants;
- per-query reader schema-version routing.

### PERFORMANCE BASELINES TO PRESERVE
- Rule-plan compilation scales with schema rule count, not data cardinality.
- Compiled `TextPattern` continues to avoid per-character allocation/backtracking.
- Entity invariant evaluation is one compiled rule over the exact semantic owner extent; no schema-name lookup or physical routing occurs per value.
- Preserve P397-P400 realization native-cost class and P435 persistent-sharing behavior.

### NEXT RECOMMENDED PASS
**PASS446 — Candidate-bound invariants -> transaction `require`:** first prove the persisted entity invariant law against exact Candidate/future-world state with hostile field updates and reopen, then introduce transaction `require` only as the same deterministic expression bound to Candidate semantics. In parallel, begin R&D for exact relation/aggregate coordinates needed by true model-wide invariants; do not add a generic fallback validator.

## PASS446 — Candidate-bound transaction requirements on common SemanticRuleExpr

### CLOSED THIS PASS
- Added passive `Transaction::require::<E>(id, SemanticRuleExpr)` preconditions. They are not callbacks and do not publish independently.
- Requirements bind to the exact proposed future relation row. `Database::preview` and `Database::commit` evaluate them against the transaction Candidate; a certified stale transaction evaluates the same condition again against the rebased Candidate before publication.
- Reused the P443-P445 `SemanticRuleExpr` compiled evaluator through a new relation-row resolver keyed by stable semantic relation-column IDs. No second precondition evaluator exists.
- Rejected duplicating every object scalar into `DatabaseState.fields`. Object scalar authority remains the canonical relation row; relation-row rule resolution maps stable semantic column IDs to that row directly.
- Added schema typechecking for relation-row rules (`validate_relation_row_rule`) so requirement expressions fail closed when they reference unknown/wrongly typed coordinates.
- Requirement evaluation is authorization-visible: every referenced field requires current `ReadField`/stronger authority; a field-free requirement requires relation-read authority. `require` cannot become a side-channel around P439 authorization.
- Requirement mutation rotates the transaction's opaque client identity; changing a precondition cannot alias an earlier retry identity. `Transaction::into_plan()` rejects requirements rather than silently dropping them.
- P444 transaction construction law remains unchanged: adaptive `Transaction::new()`, strict `Transaction::from(snapshot)` only.

### SELECTED LAW
```text
Transaction writes + deterministic require expression
                    |
                    v
          exact source Candidate
                    |
       SemanticRuleExpr over stable
       relation-row semantic columns
                    |
        must evaluate true
                    |
     certified stale transport?
                    |
                    v
          exact rebased Candidate
                    |
          must evaluate true again
                    |
                 publish
```

### HOSTILE / REJECTED
- REJECTED: evaluate `require` only on the formation/source world. It constrains the future Candidate, not the pre-write state.
- REJECTED: callback/closure validators or a second precondition engine.
- REJECTED: mirror all object scalar values into `DatabaseState.fields` just to reuse entity-rule storage; this creates an O(data) duplicate representation.
- REJECTED: bypass read authorization through boolean success/failure of a requirement.
- REJECTED: silently erase requirements when converting a transaction to a bare low-level `Plan`.

### VERIFICATION
- exact future-Candidate hostile: PASS (`age 17` rejected by `require age>=18`; `age 20` preview/commit succeeds);
- `cfmd` public surface: **51/51**;
- `cfmd-runtime`: **51/51** total across unit/async/end-to-end groups (3 + 9 + 39);
- `kernel-schema`: **12/12**;
- `kernel-validation`: **20 passed / 1 ignored**.

### OPEN — IMMEDIATE
1. Hostile stale/rebase matrix for `Transaction::require`: independent intervening change preserving condition must remain publishable; an intervening change falsifying the condition must fail before publication.
2. Add explicit restricted-session hostile proving a requirement cannot observe a field without `ReadField`/`ReadRelation` authority, including revoke-before-commit.
3. Decide whether transaction requirements should gain their own persisted canonical digest in durable client-intent metadata before externally supplied/recoverable transaction IDs become public product DX. Current Rust `Transaction` rotates its opaque generated identity whenever requirements change, so the public Rust path cannot alias changed guards, but durable first-class requirement identity remains worth formalizing before broader bindings expose explicit retry construction.
4. R&D exact relation/cardinality/aggregate/quantifier coordinates for true model-wide invariants using query/aggregate algebra rather than generic full-model fallback.
5. Final typed Rust DX for invariant/require expressions remains open; current `SemanticRuleExpr` surface is semantic substrate, not necessarily final ergonomic syntax.

### OPEN — R&D / REMOTE-READER SCHEMA-EVOLUTION DX
- reader may contain no authoritative schema definition;
- no reader-specific server contracts/annotations and no dedicated compatibility handshake;
- numeric schema epoch/version comes through ordinary metadata without `ModelRead`;
- compatibility resolves once at `Context<M>` / `Snapshot<M>` binding, never per query/row;
- no field-name/existence fallback; same spelling may carry different semantics across epochs;
- uncovered epochs fail closed;
- authoritative schema remains current-only and compatibility history remains consumer-side;
- final annotation/API syntax remains deliberately undecided; `bind/rebind` is not selected terminology.

### OPEN — DEFERRED / MANDATORY CARRY
- migration frontend Rust/Python/TMD/CLI + diagnostics;
- final Python/.NET/Studio surfaces;
- backup/restore/corruption UX and exact admin permissions when those surfaces become reachable;
- public performance/binary-size budgets;
- native Windows secure-memory expansion;
- transient/wide persistent-tree sorted bulk-builder R&D remains performance-only.

### SUPERSEDED / DO NOT EXTEND
- public `ReadContext` / any `ReaderContext`;
- `Snapshot::transaction()`;
- Context shape as security;
- host callbacks/host regex validators;
- separate transaction precondition evaluator;
- duplicate object scalar mirror solely for rule evaluation;
- name/ordinal-based rule identity;
- per-query reader schema-version routing.

### PERFORMANCE BASELINES TO PRESERVE
- transactions without `require` pay no Candidate requirement-evaluation work;
- requirement rule resolution is by stable semantic column identity against the exact candidate row, not full-model scan;
- no duplicate O(data) scalar projection was introduced;
- compiled TextPattern remains non-backtracking/no per-character allocation;
- preserve P397-P400 realization native-cost class and P435 persistent-sharing behavior.

### NEXT RECOMMENDED PASS
**PASS447 — require rebase/auth hostile closure + model-wide invariant algebra R&D.** First close stale/rebase and granular authorization matrices for the new passive precondition law. Then, if clean, formulate exact relation/cardinality/aggregate coordinates for true model-wide invariants using existing query/aggregate semantics; do not introduce a generic validator fallback.
