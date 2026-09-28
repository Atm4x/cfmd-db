# CFMD Product Roadmap

## Product thesis

CFMD's first practical product surface is a **Python-first embedded/local database facade** over the converged kernel.

The read vocabulary should stay familiar; the differentiator is the combination:

```text
familiar lazy query
+ deep domain traversal without manual join plumbing
+ exact query-result watch
+ historical contexts
+ queryable Candidate future state
+ future query delta / explanation
+ semantic inverse / undo
+ one authoritative runtime
+ live local tooling over the same protocol
```

This roadmap is derived from [`CFMD_PYTHON_FACADE_THEORY.md`](CFMD_PYTHON_FACADE_THEORY.md). That document remains the detailed design source; this file is the implementation sequence.

## Non-negotiable facade laws

1. **Types are types, not databases.** Use `db.users`, not `User[id]`, as the normal database-bound root.
2. **No hidden I/O on materialized Python objects.** Symbolic query proxies may traverse relationships; ordinary Python attribute access may not silently hit storage.
3. **Many-valued paths are explicit.** Require `any/all/match/where/count/...`; do not silently flatten ambiguous collections.
4. **Worlds are explicit.** `db` is current state, `db.at(revision)` is historical state, `db.preview(plan)` is speculative state.
5. **Shape-changing operations remain lazy.** Terminals such as `all/one/value` execute.
6. **Exact watch means exact.** If a query has no certified incremental derivative, exact watch fails with a capability reason unless the caller explicitly chooses recomputation.
7. **Plans precede advanced commits.** A Plan is an inspectable proposed transition; Candidate is a queryable future world.
8. **One authoritative runtime.** External tools never bypass revision/validation/maintained-state authority by editing database bytes independently.

## Phase 0 — repository/kernel freeze baseline — COMPLETE

Pass280 closes the global kernel hostile/refactor campaign for the declared scope.

Release-prep requirements before facade implementation:

- reproducible Rust/Lean CI;
- complete vendored dependency snapshot;
- manifest/repository completeness gate;
- current architecture/status/spec documentation;
- kernel hostile ledger and evidence-driven reopen policy.

## Phase 1 — minimal Python pet-project surface

Goal: a small local application can use CFMD without importing internal kernel crates or learning internal plan/revision ownership.

Deliverables:

- installable CPython wheels;
- `cfmd.open(...)` / create/open/close lifecycle;
- typed/generated schema surface with strong IDE/Pyright support;
- entity sets (`db.users`, `db.messages`, ...);
- `get` / `require` identity lookup;
- `match` equality filtering;
- symbolic `where` predicates;
- `select`, ordering, limits and basic aggregates;
- declared relationship navigation in depth;
- explicit `Ref[T]` loading for materialized objects;
- atomic immediate CRUD sugar implemented over the same write machinery;
- stable domain-oriented error taxonomy;
- executable tutorial and end-to-end local-app tests.

Acceptance gate: a realistic small application can create/open a durable DB, transact and query using only the product facade.

## Phase 2 — Plan / Candidate / history

Goal: expose CFMD's revision/change model as useful application behavior rather than kernel internals.

Deliverables:

- first-class `Plan` / rewrite construction;
- typed operation metadata;
- `db.preview(plan) -> Candidate`;
- query Candidate state with the same query objects used for current state;
- `Candidate.delta(query)`;
- Candidate validation/conflict surface;
- freshness checks and explicit `StaleCandidate / RebaseRequired`;
- revision/history browsing;
- history entry → inverse Plan;
- undo preview through the normal Candidate pipeline.

Acceptance gate: applications can inspect “what would the database say if I commit this?” before authoritative publication.

## Phase 3 — exact watch protocol

Goal: make reactive local applications a database capability rather than a second hand-written event architecture.

Deliverables:

- language-neutral query subscription protocol;
- revision-tagged atomic transaction batches;
- collection `QueryDelta` and scalar `ValueChanged`;
- exact-watch capability reporting;
- `watch(since=revision)` with explicit `ResetRequired` when exact replay is unavailable;
- bounded buffers/backpressure policy;
- explicit optional `mode="recompute"` escape hatch, never mislabeled as exact maintenance;
- async Python `watch()` / `values()` surface.

Acceptance gate: a committed external change can update a subscribed Python UI through exact query-result deltas without table-level manual event wiring.

## Phase 4 — local tooling and Studio

Goal: make CFMD a live state debugger for embedded applications.

Deliverables:

- app-owned local tooling endpoint (Named Pipe on Windows, Unix Domain Socket on Linux/macOS; optional localhost TCP);
- capability-token protected read-only/read-write modes;
- schema/query/revision/history/Plan/Candidate/watch protocol;
- CFMD Studio attach/discovery workflow;
- active-watch inspector;
- Candidate direct/derived impact view;
- query explainability (`logical`, `physical`, capabilities, dependencies);
- PyQt/PySide adapters over the language-neutral watch stream.

Acceptance gate: CFMD Studio can edit through Plan → Candidate → commit, and the running application receives the same revision/watch effects as an in-process write.

## Phase 5 — semantic undo, explanation and richer futures

Deliverables:

- `why_changed(query)` backed by dependency/proof/change evidence;
- finer semantic conflict/commutation checks for inverse/undo;
- partial/dependency-aware undo where justified by the rewrite calculus;
- predictable schema migration workflow;
- Candidate support for selected schema/Γ futures where the kernel contracts are strong enough;
- stronger generated typing for optional and many-valued relationship paths.

## Phase 6 — additional language surfaces

Once the runtime protocol is stable:

- Rust public application facade;
- .NET/WPF continuation (`WatchAsync`, collection adapters);
- Rust/CLI/Studio plugins against the same runtime protocol.

These are additional surfaces over the same semantics, not independent database models.

## Phase 7 — production release hardening

- backup/restore and corruption-recovery UX;
- stable compatibility/semver policy;
- API compatibility CI;
- binary-size tracking;
- watch memory/resource limits;
- observability;
- platform-specific durability certification expansion;
- upgrade/downgrade story;
- public benchmark corpus and regression thresholds;
- security review of local tooling and deployment/auth boundaries.

## Product milestone definitions

### Minimum credible pet-project milestone

The project is usable when it has wheels, durable open/create, typed schema, relationships, identity/query/projection/ordering/aggregates, deep traversal, atomic commit, useful errors, a basic exact-watch subset, Plan + preview, revision/history access, an inspector/CLI and a complete tutorial.

### Compelling pet-project milestone

The product becomes distinctly attractive when most normal application queries have exact watch, Candidate query/delta works broadly, semantic undo preview exists, PyQt/PySide integration is straightforward, and CFMD Studio can attach live to the same authoritative runtime.

## Open design questions

The detailed facade theory retains the unresolved questions around `.count()` laziness, `match` naming, boolean expression capture, optional relationship typing, exact-watch v1 coverage, ordered deltas, backpressure, Candidate persistence/rebase, tooling transport/authentication and the boundary between immediate CRUD sugar and explicit Plans.

Those questions should be prototyped and measured; they are not to be settled by aesthetic API preference alone.
