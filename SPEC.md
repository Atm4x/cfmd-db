# CFMD Project Specification

This is the concise repository-facing specification. The full normative model and append-only implementation record remain in [`docs/spec/CFMD_CORE_SPEC.md`](docs/spec/CFMD_CORE_SPEC.md). Historical pass artifacts are provenance, not the primary current-status interface.

## 1. State model

A logical CFMD revision is a coherent revisioned state, conventionally:

```text
Revision = (Schema S, SemanticEnvironment Γ, finite Model M)
```

- `S` defines structural/nominal types, relations, constraints and semantic dependencies;
- `Γ` pins meaning-bearing equality/order/canonicalization and certified semantic modules;
- `M` is the finite typed model for the revision.

Runtime publication additionally owns physical roots, maintained materializations and durable authority for that same revision. Published state must not mix components from different revisions.

## 2. Type and surface model

The logical universe is structural plus nominal identity. Core structural constructors include products, sums, option, set, bag, sequence, map and guarded recursive `μ` types. Entities/references remain nominal where identity semantics require it.

Object/document/relational forms are surface views over one typed kernel, not independent database models.

## 3. Semantic environment Γ

Meaning is revisioned data. Operations depending on semantic equality, ordering or canonicalization are evaluated against pinned `Γ`. Kernel code must not substitute incidental Rust `Eq`/`Hash`/`Ord` when a certified semantic operation is authoritative. Ordered constant filters are first-class relational operators: their ordering ID is pinned during preparation, must be domain-compatible and congruent with the input column equivalence, and is applied directly to exact maintained deltas. Product object fields derive stable per-field ordering identities only for value types with declared canonical ordering; no implicit order is invented for optional/structural values. Product field coordinates are likewise semantic rather than positional or tied to the current Rust spelling. The default durable field name is the declared field name; an explicit rename retains the prior durable semantic name, and every derived equality/order/reference/field coordinate MUST continue to use that durable name. Two stored fields of one entity MUST NOT resolve to the same durable semantic name.

Authentication/deployment of semantic modules and semantic correctness are separate obligations.

## 4. Query and change calculus

Queries are exact typed expressions over the logical model. Physical plans may specialize aggressively, but checked lowering must erase to the same logical query.

Every logical type has change semantics. Incremental execution, maintained views, exact watch and writable/rewrite paths operate through typed changes rather than ad-hoc page mutation semantics.

## 5. Transactions, Plans and Candidates

Writes are typed rewrites over revisioned state. Recoverable errors occur before authoritative publication.

At the product surface, advanced writes are exposed as **Plans** before commit. `preview(plan)` creates a queryable **Candidate** future world. Candidate validation, query delta, explanation, freshness/rebase and commit must reuse the same kernel/revision semantics rather than form a parallel transaction model.

## 6. Exact watch

Exact watch observes the **result of a logical query**, not merely “a table changed”. A commit publishes one coherent revision-tagged delta batch. If an operator lacks a certified exact derivative, exact watch fails explicitly; optional full recomputation must be an explicit caller choice.

The watch protocol is language-neutral so Python, GUI adapters, .NET and Studio can share it.

## 7. Physical/runtime model

The runtime uses prepared logical/physical metadata, exact semantic indexes, persistent/COW roots where appropriate, and maintained delta execution for supported operators.

Physical representation is not itself schema authority. The target architecture separates immutable/COW physical atoms from a certified realization root `ρ` that maps those atoms into the one current semantic model. A schema migration `M : A -> B` changes semantic authority atomically and compiles the next realization by composition (`ρ_B = normalize(M ∘ ρ_A)`); old physical atoms may remain reachable without keeping schema A alive in the current semantic world. Representation-only materialization may replace `ρ` by an extensionally equal `ρ'` under the same database revision, and physical GC is reachability from current/historical realization roots rather than a migration-progress bitmap. The current `kernel-realization` crate is the in-memory/reference calculus for these laws; durability remains on the existing full-Revision authority until that calculus is integrated and certified. P396 further requires production realization to be factorized by physical column/segment/chunk rather than per stored value: reference per-cell expressions are correctness oracles only, and hot derived coordinates must be materializable back to native-speed direct realization.

Internal `kernel-*` representations remain implementation details. Public bindings consume a compact stable runtime/facade boundary, not crate ownership/layout types.

## 8. Durability and recovery

Durability uses immutable generations, WAL/prerequisite ordering, explicit sync/publication steps, authenticated durable evidence and fail-closed recovery rules.

Publication/GC obligations are mechanically checked in Lean for the declared boundary. Real durability claims remain platform-profile-scoped.

Current certified profile remains the repository support-matrix profile; this does not imply blanket bare-metal/filesystem certification.

## 9. Distribution, trust and deployment

Replication/transport authority, trust-root/key lifecycle, signed evidence and freshness/anti-rollback are distinct from logical query semantics. Large external artifacts are authenticated metadata-first and read through bounded contracts where applicable.

## 10. Formal boundary

Lean artifacts mechanize selected publication and surface-to-kernel obligations. Source-refinement binders fail closed when proof-relevant Rust vocabulary/protocol drifts from those artifacts. They do not claim arbitrary machine-code verification.

## 11. Kernel status

The global hostile/refactor campaign is **COMPLETE / FROZEN after Pass280** for the current declared scope.

Every kernel crate has received a dedicated or grouped hostile audit proportional to its size, and the historically heavy `query/plan/semantics/durability` line was revalidated against the final workspace. `FROZEN` is evidence-driven: reopen on a counterexample, proof/authority seam, measured complexity regression, new mathematical requirement or public API/DX requirement—not for cleanup by inertia.

Current inventory: [`docs/status/KERNEL_HOSTILE_LEDGER.md`](docs/status/KERNEL_HOSTILE_LEDGER.md).

## 12. Product boundary

The active product direction is **Rust-first**. `cfmd` is the public Rust application crate; `cfmd-runtime` is the stable semantic/runtime boundary used beneath it and by future bindings; through Pass282 it owns create/open, schema/type/relation construction, snapshots, query IR and relation Plans. Python/.NET/Studio bind to it later and never call `kernel-*` crates directly.

Primary surface goals:

- explicit database-bound entity sets (`db.users`);
- familiar lazy `match/where/select/order/aggregate` operations;
- deep symbolic relationship traversal without manual join plumbing;
- no hidden I/O in higher-level materialized language objects;
- Plan → Candidate → commit workflows;
- revision/history access and semantic inverse/undo preview;
- exact async query-result watch;
- one authoritative runtime shared by the application and local Studio/tooling;
- inspectable dependencies/capabilities/explainability.

Detailed design: [`docs/api/CFMD_PYTHON_FACADE_THEORY.md`](docs/api/CFMD_PYTHON_FACADE_THEORY.md). Implementation sequence: [`docs/api/PRODUCT_ROADMAP.md`](docs/api/PRODUCT_ROADMAP.md).


## Current product surface (P284)

Rust-first productization now has an object-first domain layer over the universal `cfmd-runtime` relation/query protocol. Application writes produce `Plan` values; relation-first APIs remain available for dynamic/tooling consumers. P342 supersedes the earlier reverse-backlink model: `Ref<T>`, `Many<T>`, and `OwnedMany<T>` are first-class object relationship values, while typed edge relations are internal runtime lowering. A many-valued relationship does not require a target-side backlink. P343 adds relation-local exclusive ownership plus explicit orphan policy, P344 keeps filtered relationship selections mutable without target-object materialization, P345 freezes executor-neutral exact-watch readiness/drain, P346/P347 establish and hostile-close the zero-runtime-dependency Future/Waker path, P348 makes async direct on the watch with dependency-frontier wake filtering and observable quotienting, and P349/P350 validate the same law from CPython asyncio while preserving cancellation/loop-shutdown delivery correctness at the FFI boundary.


### Pass285 product-layer entity contract

The external Rust product layer may declare identity-bearing entities independently of kernel lifecycle carriers. `Id<T>` and `Ref<T>` are encoded through the existing typed historical-entity identity domain; they do not create hidden object I/O or a second storage model. Entity plans carry a product-level contract: identity values are unique and every strong reference resolves in the final composed plan state. Validation is identity-indexed (`BTreeMap`/`BTreeSet`), not reference-by-target scanning. Deep reference predicates lower to existing equality joins, then project and semantically distinct the root shape, giving existential path semantics without multiplicity leakage. Kernel relation/query semantics remain unchanged.

## Current product-layer status (Pass286)

The frozen kernel graph is now exposed through an object-first Rust facade with identities, required/optional references, reverse-many symbolic relationships and explicit cardinality operators. Strong-reference lifecycle authority is the next backend integration target: it must become durable/global at the kernel lifecycle layer rather than remain facade-local metadata.

## Pass287 lifecycle unification

Identity-bearing object writes now derive a complete kernel lifecycle projection before publication. External IDs remain typed historical identities for DX and deep-query equality. For kernel lifecycle, each `(object TypeId, external id)` is deterministically mapped to an internal `EntityId`; collisions fail closed.

For every identity-bearing relation touched by an object Plan, the runtime rebuilds the carrier/root set and mirrors required/optional references into schema-declared kernel fields typed as `LiveEntityRef` / `Option<LiveEntityRef>`. `Revision::build` is the single invariant authority. A surviving mirrored field referencing a deleted entity causes kernel `DanglingLiveReference`, including after runtime reopen.

Entity Plans use durable full-revision publication because their endpoint changes lifecycle/carrier/field state as well as relation data. Relation-only Plans retain the compact derived-relation path. A future optimization may add a compact mixed entity transition, but must preserve exactly the same Revision endpoint and lifecycle law.

## Pass288 mixed revision publication

Identity-bearing object transitions now publish through a first-class `kernel-plan::MixedRevisionTransitionRequest`. The request carries one already validated target Revision plus the exact relation mutations that explain every changed relation under the pinned semantic context. Lifecycle, carrier and field state may change; untouched relations may not.

The runtime applies relation mutations to a clone of the existing physical store, incrementally maintains semantic-quotient support and materialized query state, rebuilds the logical violation measure from the exact target Revision, and then publishes one atomic runtime root. `RuntimePublicationEffect` is `Incremental`; the previous full physical-store reconstruction is no longer on the entity write hot path.

Durability deliberately remains exact-full-target in P288. The WAL/idempotency intent keeps canonical target Revision bytes because lifecycle/carrier/field changes do not yet have a versioned compact durable delta codec. A future durability-format pass may encode that non-relation delta compactly, but it must decode to the identical target Revision and preserve retry/recovery semantics.

## Candidate diagnostics status (Pass291)

Candidate is a certified proposed Revision, not an advisory simulation. Construction succeeds only after the target revision has passed the same kernel revision/model validation used by publication. Product diagnostics therefore expose facts that are actually knowable before commit: invariant certification and runtime freshness (`Ready`, `Stale`, `RuntimeClosed`). Transaction-ID conflicts remain commit-time facts unless a concrete transaction identity is supplied. Candidate object queries support the same typed projection codecs as current snapshots. `CandidatePreview` is an immutable presentation summary of source/target revisions, explicit Plan effects, derived lifecycle/policy effects, and diagnostics; derived orphan deletion is visible before commit.

## History inverse status (Pass292–P293)

History is a projection of the durable causal revision-effect authority, not a second journal. A product `HistoryEntry` carries causal prerequisites, transaction identity, source/target revision and exact relation changes. Ordinary application undo is accumulated through `Database::undo(&mut Transaction, &HistoryEntry)` / `Database::undo_latest(&mut Transaction)` and therefore shares the same preview/commit control surface as entity and relationship writes. Internally the inverse remains an ordinary Plan whose relation mutations are the exact swap of the committed delta and whose optional explicit model delta is the certified durable reverse complement. `undo_plan()` remains an advanced explicit representation. Undo is compensating history, so redo is the inverse of the committed undo entry.

For new mixed transitions the durable intent stores both the compact forward `DurableModelDelta` and its exact reverse complement. `kernel-plan` recomputes both directions from the source and target Revision during preparation and rejects a mismatched complement. WAL mutation codec v11 persists this information atomically with the same transaction authority; v10 mixed records remain backward-readable but cannot manufacture a missing complement. This makes lifecycle/carrier/reference-changing object history exactly invertible across reopen without a full old-Revision snapshot or fallback path.

## Historical snapshot status (Pass294)

A committed revision is a first-class immutable product world. `Database::at(revision)` reconstructs the exact logical Revision from the live durable head and the retained Γ-REIC causal effect graph; it does not create a second snapshot journal or persist full historical Revision copies. Reversible relation effects are traversable in both directions by swapping their exact deltas, while mixed effects additionally use the persisted P293 forward/reverse `DurableModelDelta` pair. If no exact reversible path exists, historical reconstruction fails closed.

The returned `ReadContext` shares the ordinary schema/query/object read vocabulary with a live snapshot but has no Plan capability. `db.history()` is therefore the history view anchored at current HEAD, while `db.at(r)?.history()` is the causal history anchored at revision `r`. Mutation from a historical world requires a future explicit rebase/restore operation rather than an implicit checkout.

## Non-head history undo / semantic rebase (Pass295)

Historical undo now works beyond the live head when the kernel can prove transport to the current revision. `HistoryEntry::undo_readiness()` reports whether an inverse is immediately ready, certified rebased, semantically conflicting, non-reversible, unavailable, or detached from its runtime. `undo_plan()` uses the same certificate and still returns an ordinary current-snapshot `Plan`.

The proof boundary is an exact semantic write footprint. Relation writes use Γ-canonical row-class coordinates, so two independent rows in the same relation can commute without relation-wide locking. Mixed object/model writes use carrier, field-owner, lifecycle and keeps-alive coordinates. Any overlap, opaque intervening transition, or unavailable exact history fails closed. There is no generic three-way merge, last-write-wins behavior, or fallback mutation path.

## Pass296 — exact revision-tagged watch foundation

Exact watch is now implemented in the Rust product runtime for queries accepted by the maintained relational differential program. A subscription is anchored to one live Revision, materializes the query's existing `MaterializedRelPlanState` once, and thereafter advances only through exact committed relation deltas recovered from the durable causal effect chain. Each delivered event names source/target revisions and exact inserted/removed query-result rows.

The runtime publication signal is a non-authoritative wake primitive only. Durable history plus the immutable Revision remain the source of truth after wake. P346/P347 add race-free standard-library `Waker` registration owned by `PublicationNotifier`. P348 moves the Future directly onto every exact watch (`watch.next().await`) and removes the transitional `cfmd-async` wrapper crate. Pending wakers are indexed by exact relation dependencies and relation-data publication wakes only intersecting subscriptions; opaque/full/schema/liveness signals may broadcast. Output-equivalent effects advance the causal cursor without fabricating empty public events. There is still no polling, result recomputation, state-diff fallback, mandatory Tokio dependency, helper thread, or second event log. Opaque/full/schema historical transitions and unavailable causal coverage fail explicitly. Cross-process wake transport remains open and must preserve the same exact causal-delta protocol.

## Pass297 — host-provided publication notification boundary

Exact watch no longer depends on a hard-wired `Condvar` owner. `kernel-plan` now defines a wake-only `RuntimeRevisionPublicationNotifier` contract, while `cfmd-runtime` exposes the kernel-independent `PublicationNotifier` product contract plus a first-party `InProcessPublicationNotifier`. Hosts can opt into `Database::create_with_publication_notifier` / `open_with_publication_notifier` and supply a transport/event-loop specific backend without changing query/watch semantics.

The notifier is outside mutation and writer-resolution authority. It can only wake observers; every watch event is still reconstructed from the immutable Revision and durable causal effect history. Duplicate/spurious notifications therefore cannot fabricate a result transition. Concurrent-writer commute/rebase/conflict certification remains kernel-owned semantic authority and is not delegated to notification/transport providers.

Hosted CFMD should compose authentication, authorization, protocol and transport above this boundary. No database open/create call starts IPC, listens on a socket, trusts localhost, or grants an external writer capability implicitly. First-party local/network providers may be added as optional modules while application-defined providers remain possible.

## Pass298 — watch lifecycle, cancellation and bounded catch-up

A watch owns a per-subscription cancellation/wait capability that references only the wake provider, not `DurableRuntime`. Cancellation and runtime shutdown are wake-only lifecycle signals and cannot advance authoritative Revision state. Blocking `recv()` therefore terminates deterministically on explicit cancellation or final runtime shutdown without timeout polling. The public error taxonomy distinguishes this terminal lifecycle state as `WatchClosed`.

Watch backlog is not an in-memory event queue. The subscription stores one maintained query state and one anchor Revision; `WatchStatus::Lagging` reports the exact number of retained causal transitions to HEAD and each receive consumes exactly the next transition. `Current`, `Cancelled`, `RuntimeClosed` and `Unavailable` are explicit status states. Missing causal coverage, opaque transitions or non-reachable anchors fail closed; no transition may be silently dropped and no full-query recomputation may be substituted.

`PublicationNotifier` is now phrased around the required primitive `notify_waiters()`. Real publication uses that primitive only after durable Revision authority advances, while cancellation/shutdown use it solely for liveness. This prevents transport/provider APIs from treating cancellation as a synthetic publication event.


## Pass299 — hosted session/security authority boundary

Hosted authorization is now a product-runtime capability boundary, not a transport convention. A trusted host supplies an authenticated `PrincipalId` and host-controlled shared session authority; CFMD does not authenticate usernames, sockets or TLS peers in this layer. `SessionDatabase` exposes the ordinary database vocabulary under those grants. Existing embedded `Database` remains the unrestricted local capability.

Session authority propagates through derived product values. Restricted `ReadContext` values gate watch/history/write derivation; `Plan` and `Transaction` carry their creating authority into proposed future state; Candidate reads check `Read`; `HistoryEntry::undo_plan()` checks `Write`. Plans from distinct session authorities cannot be composed. `SessionDatabase::preview`, `SessionDatabase::commit`, and the advanced `SessionDatabase::commit_plan` reject intents created under a different session authority. Candidate has no publication method. Therefore endpoint code cannot bypass authorization by switching to a derived product value, object-query watch, or history inverse API.

Runtime authorization consumes one exact `PermissionSet` law. Coarse `Read`/`Write` are explicit supersets; granular relation/field/object/relationship grants authorize semantic footprints/actions. `ModelRead` is distinct from data read authority and gates full authoritative schema inspection; `SchemaMigrate` is distinct from data write authority and gates schema migration publication. Named `Role` is only an immutable permission-bundle DX surface that is flattened before enforcement; it is not a second role engine or hierarchy. Schema epoch metadata remains separately observable through `schema_revision()` so remote-reader compatibility need not imply full model disclosure.


## Pass300 — transport-neutral hosted protocol/session foundation

`cfmd-protocol` is a separate product crate above `cfmd-runtime`. The dependency direction is one-way: protocol may depend on runtime, while runtime and kernels must not depend on protocol or transport crates. Protocol ingress is bound to an already-authorized `SessionDatabase`; raw unrestricted `Database` is not a protocol input.

The protocol owns its external DTO vocabulary: recursive values/rows, query AST, snapshot selectors, relation mutations, history records, requests/responses, protocol version and stable error codes. Runtime/kernel Rust types are converted at the boundary rather than exported as protocol contracts.

Unary operations currently include current revision, query at HEAD or an exact historical revision, revision-anchored history, and relation commit. Commit requests MUST carry `base_revision` and transaction identity. Dispatcher construction of the ordinary restricted `Plan` MUST compare its actual base revision to the requested base and fail closed on mismatch before mutation publication. Authorization remains runtime authority and MUST NOT be reimplemented as a transport convention.

Protocol ingress MUST apply resource bounds before runtime execution. P300 bounds query node/depth complexity, relation-mutation count, commit row count, row width and recursive value complexity. Internal, recovery and invariant diagnostics MUST be sanitized before crossing the untrusted protocol boundary.

P300 defines no wire codec, listener, IPC/TCP stack or authenticator. Concrete hosted transports remain adapters above this crate.

## Pass301 — exact hosted watch subscription protocol

Hosted watch is now a protocol-owned subscription vocabulary over the existing exact runtime watch. `OpenWatch` MUST validate the protocol query under the same ingress limits and MUST construct the restricted live `QueryWatch`; historical targets remain non-watchable. The opening response carries a session-scoped `SubscriptionId` and the exact initial query result/revision. `NextWatch` MUST consume exactly one runtime watch transition and return source/target revisions plus inserted/removed protocol rows; a real revision transition with an empty result delta MUST still be observable as an event.

`CancelWatch`, `CloseWatch`, and `CloseSession` are lifecycle/wake operations only. They MUST NOT mutate database Revision or create a second event authority. Cancellation MUST remain able to wake a blocked `NextWatch` without taking the maintained query-state consumer lock. A hosted session MUST bound concurrent maintained subscriptions, and a subscription MUST reject a second in-flight consumer rather than accumulating an unbounded waiter queue. Closing a session MUST cancel all outstanding subscriptions so a transport disconnect can terminate blocked work deterministically.

Protocol version 2 includes this watch vocabulary. P301 still defines no wire encoding, framing, listener, IPC/TCP/TLS transport, or authentication mechanism.


## Pass302 — canonical wire framing and protocol negotiation

`cfmd-protocol::wire` defines canonical binary framing above the transport-neutral hosted semantics. Wire framing version 1 uses a fixed 20-byte header carrying magic, framing version, frame kind, zero-reserved flags, request identity and payload length. A transport MUST be able to validate that header and configured payload bound before allocating/reading the payload. Payloads use deterministic big-endian scalar encoding, explicit discriminants and bounded length prefixes; malformed/truncated/trailing/non-canonical encodings fail closed.

A wire session MUST complete `ProtocolHello` negotiation before ordinary hosted requests. The server selects only a hosted protocol version inside the offered client range and returns the intersection of client-requested and server-supported capability bits. Current wire v1 negotiates hosted protocol v2. Request ids are correlation metadata only and MUST NOT become transaction/database authority.

The wire decoder MUST enforce independent frame/payload, collection, string, recursive-node and depth limits before unbounded allocation. Unknown frame/request/value/status/error tags are invalid for the negotiated schema; fields are not silently skipped. Protocol evolution occurs through explicit version/capability negotiation rather than permissive interpretation of mutation/security-bearing messages.

`WireHostedSession` dispatches only into the already-authorized `HostedSession`. Framing/negotiation cannot grant permissions, mutate Revision, certify writer commutation, authenticate peers or start transport infrastructure. Concrete IPC/TCP/TLS/QUIC implementations remain adapters above this layer.

## Pass303 — transport-neutral hosted server composition

**[HOST COMPOSITION LAW]** `cfmd-host` is the first server-composition layer above `cfmd-protocol`. It owns no concrete transport. A transport adapter may supply authentication evidence and move wire frames, but it is never handed unrestricted `Database` authority.

**[AUTHENTICATION / AUTHORIZATION SPLIT]** Authentication maps provider-specific evidence only to `PrincipalId`. Authorization independently maps that principal to `PermissionSet`. Only `cfmd-host` constructs the runtime `Session` and restricted `SessionDatabase`. Transport metadata, localhost, socket/process identity, or a connection identifier cannot synthesize grants.

**[RESOURCE LAW]** Connection admission is bounded before authentication work is accepted into an active hosted slot. Per-connection concurrent frame execution is separately bounded. Watch cancellation and server/connection close remain lifecycle operations; they do not acquire writer authority.

**[LIFECYCLE LAW]** Disconnect or explicit host shutdown closes the associated `HostedSession`, wakes/cancels blocked watch work, releases connection capacity exactly once and rejects subsequent frames/new connections. Already-admitted unary database operations may finish through the ordinary runtime transaction semantics; shutdown does not invent rollback semantics.

**[EXTENSION LAW]** Concrete local IPC, TCP/TLS, QUIC and application-specific transports remain providers above this layer. Security providers are replaceable, but semantic writer commute/rebase/conflict certification remains kernel authority and cannot be supplied by a transport or authentication provider.

## Pass304 — dynamic hosted security lifecycle

A restricted session MUST carry one shared runtime authority identity across all derived product values. Grant refresh MUST be observed by already-created values at their next authority check; revocation MUST be monotone and MUST prevent further authorized derivation/commit through those values. Refresh/revoke are security-lifecycle transitions only and MUST NOT mutate database Revision.

Authentication context distinguishes explicitly bound and unbound channels. Binding metadata is evidence for the configured authenticator only and MUST NOT itself grant database permissions. Authorization grants MAY carry an expiry deadline. The host MUST expose deadline scheduling without requiring a background polling loop; expiry tears down the hosted session and its blocking subscriptions. Graceful drain MUST reject new connections while preserving already-admitted connection semantics until close.


## Local transport provider boundary (P305)

The first-party local transport is an adapter above `cfmd-host`; it is not a database authority. A Unix-domain-socket endpoint may harden OS access with private permissions, but locality/path possession cannot establish `PrincipalId`, permissions, writer commutation or Revision authority. Authentication evidence is length-bounded before allocation and then passed unchanged into the configured host authenticator.

Transport concurrency is bounded independently from database semantics: each connection has a fixed worker count capped by the host in-flight limit and a bounded response backlog. The reader remains independent of blocked request workers so peer EOF deterministically closes the hosted connection and cancels blocked watch work. Wire request ids correlate responses; resource pressure yields stable protocol resource errors rather than an unbounded queue.

P305 supplies a Unix-domain-socket provider only. Windows named pipes and network transports are sibling providers; they must conform to the same `cfmd-host` / `cfmd-protocol` security and lifecycle contracts rather than extending kernel semantics.

## P306 single-file physical container law

A future `*.cfmd` backend MUST recover from the database file alone. Recovery correctness MUST NOT depend on a sibling temp/delta/sidecar file. New immutable generation bytes MUST be fully durable before a root switch is published. Root publication uses alternating independently checksummed pages; an invalid/torn newer slot may be ignored, while a valid newer slot is authoritative and corruption of its referenced generation MUST fail closed rather than silently roll back. When both root slots are valid they MUST be consecutive in sequence and generation and the newer parent digest MUST equal the older generation digest. Generation construction and extraction MUST support streaming I/O rather than requiring a second whole-generation memory image.

## P307 single-file WAL / generation-rotation law

A live single-file generation owns one WAL region beginning at the root-recorded `journal_offset` and `journal_first_lsn`. While the journal is open, WAL durability is exactly the existing CFMD prepare/commit frame protocol. Before any later generation bytes may be appended, the current WAL is synchronized and the root is republished as **sealed**, recording an exact `journal_end` and `journal_next_lsn`. Only after that sealed root is durable may generation `G+1` be appended and synchronized; publication of `G+1` then installs a new root whose `journal_first_lsn` equals the sealed predecessor's `journal_next_lsn`.

Crash recovery laws:

1. torn active WAL tail -> existing WAL scanner exposes only complete durable commits and truncates to its last-good frame boundary;
2. torn seal root -> previous open root remains authority;
3. valid sealed root + incomplete/orphan next generation -> validate the sealed WAL prefix, discard bytes after `journal_end`, republish the same generation as open, and continue at its original first-LSN sequence;
4. valid new-generation root -> new generation and its carried first LSN are authority;
5. no recovery path infers a WAL/generation boundary from file magic, EOF heuristics, or a temp/sidecar file.

The journal and generation authority are therefore one self-contained `*.cfmd` crash protocol, while transaction semantics remain owned by the existing WAL implementation.

## P308 single-file store authority law

A single-file `DurableRevisionStore` uses the same logical commit authority as the directory-backed store: PREPARE/COMMIT publication, retry/idempotency state, causal revision effects, migration complements, and durable head advancement remain owned by the existing store/WAL state machine. Physical checkpoint rotation may replace only the backing publication mechanism: checkpoint + metadata + prepared-cut capsule are committed as one P306 generation after the P307 WAL is durably sealed, and the next journal begins at the exact carried LSN. Recovery must reconstruct the canonical in-memory store state from the published generation plus its active WAL tail; it must never infer missing state from sidecars.

Feature families whose mutable authority has not yet been embedded into the container (replication journal, streaming checkpoint/shadow WAL, external freshness, physical compaction) are unavailable fail-closed. A single-file backend must not silently create directory-format sidecars as a compatibility fallback.

## P309 single-file replication lane law

For a single-file store, durable replication-authority records are auxiliary records in the same physical WAL LSN sequence as transaction PREPARE/COMMIT records. An auxiliary replication record may change replication membership/consensus authority but **must not** change the linear durable database Revision. Recovery must validate the outer WAL frame first, replay transaction records into the ordinary revision scanner, and replay replication payloads through the existing replication-journal codec. At synchronous checkpoint rotation, all replication authority accumulated since the previous generation is folded into the generation `ReplicationAuthority` section; the newly opened WAL starts an empty live replication lane. No sibling replication journal or second append authority is permitted.

Pinned-cut streaming publication is not equivalent to ordinary LSN-contiguous generation rotation. A future single-file streaming transition must prove that the WAL suffix carried from the pinned cut into the new generation is exact and recovers the current durable endpoint. Until that overlap/carry-forward relation is represented and validated by root authority, single-file streaming checkpoint remains fail-closed; implementations must not weaken `new.first_lsn == old.next_lsn` for ordinary rotations, scan for magic boundaries, or buffer an unbounded shadow log as a substitute.

## P310 durability backend separation law

Logical durability authority is independent of physical representation. `DurableRevisionStore` owns the semantic checkpoint/WAL/retry/idempotency/causal state machine; exactly one `DurabilityBackend` owns physical layout resources. Selecting `Directory` versus `SingleFile` cannot itself advance `durable_head`, create a commit, or change causal authority. A backend may advertise physical capabilities (currently streaming checkpoint, external freshness, physical compaction); rejecting an unsupported capability must occur before mutating semantic store state. Replication-authority persistence is delegated to the selected backend while the replication state machine remains common.

No semantic code may infer a physical capability from an optional sidecar/container field. New physical layouts must enter through the backend boundary and must reuse the same prepare/commit/recovery authority protocol.

### Single-file pinned-cut streaming authority (P311)
A single-file streaming checkpoint may publish `checkpoint@C + WAL[C..H]` as the next generation only when the carried WAL suffix is byte-complete, LSN-contiguous, cleanly decodable from the pinned checkpoint/prepared seeds, and recovers exactly the already-durable head `H`. The old journal is sealed before any generation bytes become eligible for publication. The new root is written only after the carried suffix has been copied, synced, and rescanned. Ordinary generation rotation still requires `new.first_lsn == old.next_lsn`; overlapping LSN ranges are legal only for this certified carry-forward transition and must retain the exact old endpoint `next_lsn`. No sidecar shadow WAL is used by the single-file backend.

## P312 single-file freshness and physical-compaction parity law

External freshness MUST be derived from backend-neutral authoritative material, not from directory filenames. The material is `(generation binding, generation digest, bounded WAL source, first LSN)`. For single-file storage the generation digest is the published root generation digest and the WAL source is exactly the root-authoritative journal interval; unpublished/orphan bytes beyond a sealed journal boundary MUST NOT participate. A freshness-aware reopen MUST reject rollback/truncation relative to the external authority, and physical relocation MUST preserve the same freshness material.

Single-file compaction is a physical relocation, not a logical checkpoint or new database generation. The backend MAY copy the current published generation and sealed live WAL into reclaimable space only when the destination does not overlap the still-authoritative source. Copied bytes are non-authoritative until all generation/WAL digests and exact recovery endpoint checks succeed and a certified relocation root is durably published. That relocation root MUST preserve logical generation, Revision endpoint, generation digest, parent digest, WAL first/next LSN, WAL byte length, prepared authority and replication authority; only physical offsets/root sequence may change. Tail truncation and live-WAL reopen occur only after publication. If safe non-overlapping relocation is unavailable, compaction is a correct no-op rather than an in-place overwrite.



## P313 unified product construction law

Product database construction MUST resolve physical storage before entering `kernel-plan`, then use the same `DurableRuntime` create/open/recovery semantics independent of backend. `Storage::Auto` resolves a new path to the parity-complete single-file backend, an existing regular file to single-file, and an existing directory to the directory backend; explicit storage selection overrides this resolution. `Database::create/open` and `DatabaseBuilder::{create,open}` MUST converge on the same implementation path. Runtime publication notification may be configured during database construction/open because it is a database-runtime wake capability; authentication, authorization and transport hosting MUST remain above the opened `Database` and MUST NOT become file-creation semantics.

### P314 storage encryption

Single-file product databases can opt into CFMD AE v1 using AES-256-GCM-SIV. Encryption mode (`Direct` versus provider-backed `Wrapped`) is orthogonal to the selected AEAD algorithm; AES-256-GCM-SIV is the current backend, while the storage API and persisted algorithm identifier do not encode key-acquisition mode. HKDF-SHA256 separates WAL and immutable-section keys from an external 256-bit master key; per-database salt and the algorithm identifier are public format metadata. Section/WAL physical identities are AEAD-associated data, encrypted stores require the exact key on open, rewrap cannot change the persisted AEAD algorithm, and authentication failure is corruption rather than rollback/fallback. Directory encryption remains fail-closed until parity is implemented.

### P315 AE v1 productization law

Encrypted single-file sections use a versioned 64 KiB authenticated-chunk envelope. Every chunk is independently AES-256-GCM-SIV authenticated before plaintext from that chunk is released; chunk AAD binds generation, section kind/ordinal, chunk index, total plaintext length and chunk length. `copy_section_to` decrypts with bounded chunk memory. Pre-release P314 whole-section section envelopes are not retained as a compatibility surface; unsupported encrypted section layouts fail closed. WAL nonce allocation uses a random 80-bit namespace plus a 16-bit in-namespace counter, rotating the namespace after 65,536 messages; this removes per-frame CSPRNG calls while retaining AES-GCM-SIV misuse resistance as defense in depth. Product key acquisition may be delegated to `EncryptionKeyProvider` at create/open, and external-freshness-aware single-file open uses the same encryption configuration rather than a plaintext-only probe.


### P316 wrapped-DMK / key-rotation law

Provider-backed encrypted single-file databases MUST generate a random per-database 256-bit DMK and MUST treat provider key material as a KEK, not as the data-encryption master key. The persisted key envelope binds provider key ID/epoch, database key epoch and publication sequence under authenticated wrapping. The current pre-release single-file header publishes wrapped-key metadata through two fixed slots: the inactive slot is written and durably synced before it can become authoritative, and recovery chooses the highest valid publication sequence. Rewrapping MUST preserve the DMK and MUST NOT rewrite generation/WAL ciphertext. A valid newer key slot is authoritative; authentication/provider mismatch MUST fail closed rather than fall back to an older valid slot. Pre-release pass-to-pass layouts are not a compatibility surface.

### P317 key-authority / pre-release format law

Until CFMD declares a released on-disk compatibility boundary, internal pass layouts MUST NOT create legacy-reader branches merely because an R&D pass changed the header. The single-file header/root/generation records use one current `FORMAT_VERSION`; incompatible pre-release snapshots may fail closed. Provider-backed key authority additionally carries a non-zero minimum accepted database-key epoch. Opening a fully valid wrapped-key header below that external floor MUST fail closed before DMK unwrap, so a provider that durably advances the floor can reject complete-header rollback even when the old KEK still exists. The floor is external authority, not bytes trusted from the database file itself.


### P319 bounded-memory generation publication law

Single-file generation publication MUST stream section bytes directly to their final offsets and MUST NOT construct whole encrypted sections or generations in memory. Encrypted payloads are sealed one 64 KiB AEAD chunk at a time. Because ciphertext section digests are known only after payload emission, the current pre-release generation layout stores the section descriptor table as a footer after all section data. The generation header records the footer offset/length and total generation length. The generation digest is accumulated in physical byte order during the same write; no whole-generation reread is required. Root authority remains unchanged: the streamed generation is non-authoritative until fully written, synced and referenced by a durably published root.

### Product transaction composition and transport

Rust product code groups atomic mutations with `Transaction::new()`. The transaction starts unbound; its first database mutation establishes only the formation provenance/authority of the exact internal effect. Compatible later HEAD movement is certified and transported automatically by the kernel change algebra. `Transaction::from(snapshot)` is the explicit strict-snapshot form: the supplied world is part of caller intent and any HEAD movement makes publication stale. Publication authority remains the database (`db.preview(&tx)` / `db.commit(&tx)`), while ordinary collection CRUD mutates the passive transaction via `&mut tx` without exposing `Plan` or `TransactionId`.

The same product law applies to relationship mutation. `Many<T>`, `OwnedMany<T>` and relationship selections MUST accumulate ordinary attach/detach/move/delete operations directly into `&mut Transaction`. If a transaction was already bound by an earlier operation, relationship evaluation MUST use that same formation world rather than silently compose effects evaluated at another revision. A filtered relationship selection is therefore re-evaluated against the transaction formation context before its identity-only mutation is lowered. Advanced tooling MAY request the exact low-level effect through explicitly named `*_plan` methods; ordinary application mutation MUST NOT require `Plan` plumbing.

A newer global revision is not by itself a conflict. Runtime certification classifies the already-formed exact effect against intervening causal effects using Γ-canonical coordinates and action laws. `TransactionReadiness` exposes `Ready`, `Rebasable`, or `Conflict` for diagnostics, while ordinary commit performs the same certification automatically. User mutation code MUST NOT be re-executed on a newer snapshot as an implicit retry. Unknown, opaque or coordination-required overlap fails closed.

### Pass357 stable semantic retry identity

Durable retry identity distinguishes one client semantic intent from its concrete realization on a particular source/target revision pair. For relation-data, relation-rewrite and mixed-revision intents, two realizations are the same client intent only when their canonical forward mutation payload, semantic revision/module authority and rewrite identity where applicable are equal; source/target revision ids and mixed-revision recovery complements are realization metadata. Full revision replacements, schema migrations and causal resolutions remain exact because their endpoint/parent identity is part of the operation.

The exact durable realization is still retained in WAL/checkpoint and remains recovery authority. The semantic retry comparison merely allows the same `TransactionId` to recognize a certified transported realization as the same request. A retry after successful transported publication MUST return `AlreadyCommitted` with the actually committed target revision. A different forward intent under the same transaction id MUST remain `TransactionConflict`. No last-write-wins or payload-blind transaction-id acceptance is permitted.

## Product transaction / kernel-change convergence (Pass356)

A product transaction transport certificate MUST preserve the action law attached to each exact semantic write coordinate. Runtime history MUST NOT reduce `EnsurePresent`, `EnsureAbsent`, idempotent assignment or future commutative-add semantics to an untyped touched-coordinate set when certifying forward intent transport. `StrongCommute` and `SameIdempotentIntent` may cross intervening exact effects; `DefiniteIntentConflict` fails as a conflict; `Unknown` requires coordination and fails closed unless a registered residual law is available. Bag multiplicity MUST NOT be represented as boolean set presence. Entity identity/payload coupling MUST remain conservative until an exact identity-to-payload action law is available. Historical inverse rebase remains a distinct operation and MAY deliberately use stricter overlap semantics than forward intent transport.

Pass397 performance closure: the reference per-cell realization is no longer the production granularity. `kernel-realization` now has factorized field-column rules: one physical column atom and one semantic realization rule cover all entity values of a field. Verified migration composition normalizes direct copies/constants and direct `i64 -> f64` into compiled rules. On 100k values, cutover composition is ~46-51 µs with 3 atoms/3 dependencies instead of O(N) metadata, and normalized derived point reads measure ~0.85-0.98x direct native reads in warm release runs; full-column materialization is ~14-16 ms. Relation rewrites remain fail-closed until an equivalent stable-column/segment compiler exists.
