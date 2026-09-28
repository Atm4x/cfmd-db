# Project Status

## Kernel state

The CFMD global kernel hostile/refactor campaign is **COMPLETE / FROZEN after Pass280** for the declared scope. Frozen kernels reopen only on concrete evidence: correctness counterexample, proof/authority seam, measured complexity regression, new mathematical requirement, or a product requirement that cannot be implemented cleanly above the kernel.

The historical problem ledger is closed for the declared scope; the current audit inventory is in [`KERNEL_HOSTILE_LEDGER.md`](KERNEL_HOSTILE_LEDGER.md).

## Active phase — Rust product runtime

Pass281 began the external product layer with `crates/cfmd-runtime`; Pass282 extended it through database creation/schema authority; Pass283 added typed relation/query handles; Pass284 adds the first object-first Rust domain mapping and makes application writes produce `Plan` values directly.

The architecture is intentionally Rust-first:

```text
Rust application API / Python / .NET / Studio
                    |
                cfmd-runtime
                    |
              kernel-* crates
```

`cfmd-runtime` owns stable identifiers, values, query IR, snapshots, Plans and product errors. Internal kernel types remain private implementation details.

### Implemented through P284

- `Database::open`;
- immutable revision snapshots;
- facade-owned ID/value vocabulary;
- scan/filter/project/join/distinct/TopK query IR;
- `PreparedQuery` compile-once execution;
- Bag/Set result semantics;
- relation insert/remove `Plan`;
- durable atomic commit with explicit transaction identity and stale-plan rejection;
- end-to-end durable facade regression;
- facade-owned recursive `Type`, primitive equivalence/ordering contracts and relation schema builder;
- `Database::create` without kernel construction types;
- typed-empty physical relation bootstrap (no sentinel seed rows);
- read-context schema introspection;
- semantic-ID collision rejection across facade namespaces;
- schema-checked typed relation/field handles;
- equality predicates that inherit declared relation equivalence semantics;
- typed scalar/tuple projections and typed cardinality terminals;
- extensible `ValueCodec`/`RowCodec` with typed Plan insert/remove helpers.

### Immediate roadmap

1. identity/reference/cardinality contracts on top of the P284 object model, then safe deep-path Rust query DX;
2. generated/static domain modules over the typed handles;
3. Plan sealing + Candidate preview/query/commit;
4. historical contexts/history/inverse;
5. exact watch protocol;
6. language bindings and local tooling over the same `cfmd-runtime` authority.

The detailed roadmap is [`../api/RUST_API_ROADMAP.md`](../api/RUST_API_ROADMAP.md) and [`../api/PRODUCT_ROADMAP.md`](../api/PRODUCT_ROADMAP.md). Python UX remains specified by [`../api/CFMD_PYTHON_FACADE_THEORY.md`](../api/CFMD_PYTHON_FACADE_THEORY.md), but Python does not own database semantics.

## Pass293 product status

- Durable causal history projection through `Database::history()`: COMPLETE FOUNDATION.
- Product `HistoryEntry` inspection (transaction/source/target/prerequisites/relation changes): COMPLETE.
- Exact live-head inverse as ordinary `Plan -> Candidate -> commit`: COMPLETE for relation and newly written mixed object effects.
- Entity creation/deletion, lifecycle/carrier changes and strong-reference rewrites: exact durable inverse complement persisted and validated across reopen.
- Redo semantics: undo of the committed compensating history entry; no separate mutation path or redo stack.
- Legacy mixed records without complements: backward-readable and fail closed as `ComplementRequired`.
- OPEN: non-head rebase/conflict calculus; exact watch follows on the same revision/effect vocabulary.

## Pass295 product status

- `HistoryEntry::undo_plan()` supports certified non-head undo over the current HEAD.
- `HistoryEntry::undo_readiness()` exposes ready/rebased/conflict/non-reversible/runtime/unavailable state.
- Kernel rebase authority uses exact Γ-canonical relation classes plus carrier/field/lifecycle/keeps-alive coordinates.
- Independent changes inside the same relation and across distinct object entities can commute; same semantic-coordinate writes conflict.
- Opaque/full/schema/legacy intervening effects fail closed rather than entering generic merge or replay fallback.
- Rebased inverse remains an ordinary `Plan -> Candidate -> commit`; no second undo mutation pipeline exists.
- NEXT: richer conflict/explain surface and exact query-result `watch()` can now share the same revision/effect/footprint vocabulary.

## Pass296 product status

- Exact query-result watch: FOUNDATION COMPLETE for maintained relational queries.
- Product surfaces: raw `ReadContext::watch`, object `.watch()`, and typed projected-object `.watch()`.
- Events are revision-tagged and carry exact inserted/removed result rows.
- Incremental authority is the existing `MaterializedRelPlanState` differential ExecGraph; no per-event full query replay/diff exists.
- Blocking `recv()` is event-driven through a std `Condvar` publication wake; `try_recv()` is non-blocking and no Tokio dependency is required.
- Durable causal history remains the sole transition authority after wake; the signal itself contains no semantic state.
- Historical `db.at(...)` snapshots are read-only and cannot create a live watch.
- OPEN: cross-process wake transport for external Studio/tools and async-runtime adapters over the same core watch protocol.

## Pass297 product status

- Publication wake transport is now abstract: COMPLETE FOUNDATION.
- `PublicationNotifier` is a Rust product contract; `RuntimeRevisionPublicationNotifier` is its kernel wake boundary.
- First-party in-process provider: `InProcessPublicationNotifier`.
- Host-provided provider installation is explicit through create/open APIs; no listener/server is started implicitly.
- Notification providers have no state/mutation authority; watch still consumes durable causal effects after wake.
- Semantic multi-writer resolution remains kernel-owned (Γ-canonical footprints + certified rebase/conflict), not provider-extensible correctness.
- Lean 4.34.0 gate restored; notification non-authority law is mechanized.
- OPEN: watch cancellation/lag/close lifecycle; hosted protocol/auth/transport composition; optional first-party IPC/network providers.

## Pass298 product status

- Exact watch lifecycle: COMPLETE FOUNDATION.
- Cloneable `WatchCancellation` and `.close()` interrupt blocking `recv()` without polling/timeouts.
- `WatchClosed` is a distinct terminal product error rather than a generic recovery failure.
- Last-runtime shutdown wakes blocked subscriptions; the wait handle does not keep `DurableRuntime` alive.
- `WatchStatus` exposes current/lagging/cancelled/runtime-closed/unavailable states and exact pending transition count.
- Subscription memory is O(maintained query state), not O(backlog): historical lag remains in durable causal authority and is consumed one transition at a time.
- Lean proves cancellation/shutdown wake cannot alter authoritative Revision.
- NEXT: hosted session/principal/capability/auth boundary; concrete IPC/TCP remain adapters. Single-file physical storage remains an orthogonal storage-provider project.

## Productization update — Pass299

`cfmd-runtime` now exposes a transport-independent hosted session boundary: trusted host code issues `Session { PrincipalId, PermissionSet }`, while `SessionDatabase` and derived product values enforce `Read`, `HistoricalRead`, `HistoryRead`, `Watch`, and `Write`. Authority propagates through `ReadContext`, `Plan`, `Candidate`, and history inverse values; authentication/network transports remain outside the runtime. Embedded `Database` remains the explicit unrestricted local capability.


## Productization update — Pass300

A new `cfmd-protocol` crate now owns the transport-neutral hosted ingress above P299 `SessionDatabase`. Protocol callers use protocol-owned query/value/history/mutation DTOs and cannot receive raw runtime/kernel capabilities. Unary current/historical query, anchored history and relation commit are implemented; commits bind explicit base revision + transaction identity and preserve runtime authorization/idempotency. Resource limits and sanitized error mapping are enforced before an untrusted transport can reach runtime execution.

NEXT: protocol watch/event-stream lifecycle and request/subscription identity, then explicit wire-codec/version negotiation. Concrete IPC/TCP/TLS remain optional adapters; single-file physical storage remains an orthogonal storage-provider line.


## Productization update — Pass301

`cfmd-protocol` protocol version 2 now exposes exact watch subscriptions over the existing maintained runtime watch. `OpenWatch` returns a session-scoped subscription id and initial result; blocking `NextWatch` emits one source/target-revision exact result delta; status/cancel/close expose P298 lifecycle. Subscription state is bounded per hosted session, cancellation can interrupt an in-flight consumer, and `CloseSession` cancels all subscriptions for deterministic transport disconnect cleanup. No protocol-side polling/event log or wire transport was introduced.

NEXT: explicit wire codec/framing and protocol version/capability negotiation. IPC/TCP/TLS/auth implementations remain adapters above the hosted session boundary; single-file physical storage remains orthogonal.


## Productization update — Pass302

Hosted semantics now have a canonical transport-neutral wire representation. `cfmd-protocol::wire` supplies fixed-header framing, request correlation, hosted protocol/capability negotiation and bounded deterministic encoding for all P300/P301 DTOs. Header admission checks payload length before payload allocation; the decoder additionally bounds collections, strings, recursion depth and total decoded nodes.

Wire v1 negotiates hosted protocol v2. `WireHostedSession` refuses requests before negotiation and then routes decoded requests through the existing restricted `HostedSession`, so framing cannot bypass P299 permission authority. Unknown tags/versions/trailing bytes fail closed and no permissive unknown-field semantics are used. Concrete local/network transports remain open adapters; no listener or networking dependency was added.

## Pass303 — hosted server composition boundary

The product stack now has an explicit transport-neutral host layer: `cfmd-host`. It composes authentication, authorization, restricted runtime sessions and canonical wire handling without owning sockets/listeners. Active connections and concurrent frames are bounded; disconnect/server close terminates session-scoped watch work. Concrete IPC/TCP/TLS providers remain separate follow-up components.

## Pass304 — dynamic hosted security lifecycle

Hosted grants are now shared runtime authority rather than immutable copies. Refresh/revoke affects already-derived product values, expiry can be scheduled exactly by an external event loop, channel binding is explicit authenticator evidence, and graceful drain is distinct from immediate shutdown. Runtime/protocol/host regressions cover stale Candidate authority, watch cancellation on grant removal, expiry wake-up and drain semantics.

NEXT: transport-provider conformance can now begin without changing security semantics; first-party local IPC/TCP-TLS remain optional adapters. Single-file physical storage remains an orthogonal storage-provider R&D block.

## Pass306 historical product milestone

- Single-file `*.cfmd` physical container: FOUNDATION COMPLETE.
- One-file authority publication uses append/sync generation then alternating 4 KiB root-slot sync; no sibling temp/sidecar is recovery authority.
- Generation and individual section SHA-256 verification: COMPLETE.
- Unpublished generation and torn-root crash behavior: regression-covered.
- Checksum-valid published root + corrupt generation: fail-closed, no rollback fallback.
- Root sequence/generation/parent-digest chain validation: COMPLETE.
- Streaming generation write and section copy/read: COMPLETE FOUNDATION.
- Lean single-file authority model: PASS.
- At P306 the live store integration and reclamation work were still open; P307–P312 subsequently close the WAL, store, replication, streaming, freshness and compaction physical layers.

## Pass309 historical product milestone

- Single-file replication authority: COMPLETE for live WAL + synchronous checkpoint rotation; no `replication.cfre` sidecar is used.
- Replication frames share the one physical WAL LSN stream but remain logically auxiliary: replay cannot advance the linear database head.
- Replication authority survives reopen both before and after generation rotation from one `*.cfmd`.
- At P309 pinned-cut streaming, external freshness and physical compaction were still fail-closed payers. P311 closes certified carry-forward streaming; P312 closes freshness and compaction parity.

## Pass310 durability architecture status

- Physical durability ownership is now separated from semantic store authority through one `DurabilityBackend`; the store no longer carries `directory + lock + Option<SingleFileContainer>` state.
- Directory and single-file backends own their physical resources and expose explicit physical capabilities. Semantic code asks for capability support rather than inferring behavior from layout presence.
- Replication-authority persistence is delegated through the backend boundary; replication semantics remain common.
- At P310 single-file freshness/compaction remained capability-gated while the backend separation was established.
- Full durability regression at that milestone was green at 169/169.
- P311/P312 build on this boundary rather than reintroducing layout branches into semantic durability code.

## Pass311 single-file streaming status

- Certified pinned-cut carry-forward streaming: COMPLETE for the single-file backend.
- Post-cut transaction and replication WAL suffixes are copied, synced and rescanned to the exact current durable endpoint before root publication.
- Ordinary LSN-contiguous rotation remains a distinct root law; arbitrary overlap remains corruption.
- No shadow-WAL sidecar or unbounded RAM log is introduced.

## Pass312 single-file parity status

- External freshness/anti-rollback: COMPLETE for single-file live WAL, synchronous rotation, streaming carry-forward and reopen through backend-neutral freshness material.
- Physical compaction/space reclamation: COMPLETE as verified in-file relocation of the current generation + sealed WAL; no temp file or sidecar authority.
- Compaction preserves logical generation, Revision, generation/WAL digests, prepared/replication state and external-freshness material.
- Directory and single-file backend capability surface is now parity-complete for streaming checkpoint, external freshness and physical compaction.
- Full `kernel-durability` regression: 172/172 PASS.
- NEXT: expose the parity-complete single-file backend through the product `DatabaseBuilder`/create-open surface, then add encryption-at-rest as an orthogonal AEAD/key-management storage layer.



## Pass313 product construction status

- Single-file physical backend parity from P312 is now exposed through the normal `Database` runtime path.
- `DatabaseBuilder` is the canonical construction/open surface; new paths default to single-file through `Storage::Auto`, while existing directory stores remain discoverable and explicit storage selection remains available.
- `Database::create/open` are thin sugar over the builder rather than an independent legacy path.
- Hosting remains lifecycle composition over an opened database; `DatabaseHostingExt` supplies `db.host(...)` from `cfmd-host` without moving authentication/authorization into `cfmd-runtime`.
- NEXT: AEAD encryption-at-rest/key hierarchy as an orthogonal storage codec, not a new durability protocol.
