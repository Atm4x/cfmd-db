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

Meaning is revisioned data. Operations depending on semantic equality, ordering or canonicalization are evaluated against pinned `Γ`. Kernel code must not substitute incidental Rust `Eq`/`Hash`/`Ord` when a certified semantic operation is authoritative.

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

The active product direction is **Rust-runtime first**. `cfmd-runtime` is the stable product boundary; through Pass282 it owns create/open, schema/type/relation construction, snapshots, query IR and relation Plans. Python/.NET/Studio bind to it later and never call `kernel-*` crates directly.

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

Rust-first productization now has an object-first domain layer over the universal `cfmd-runtime` relation/query protocol. Application writes produce `Plan` values; relation-first APIs remain available for dynamic/tooling consumers. Next: explicit identity/reference/cardinality contracts and safe deep paths.


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

Candidate is a certified proposed Revision, not an advisory simulation. Construction succeeds only after the target revision has passed the same kernel revision/model validation used by publication. Product diagnostics therefore expose facts that are actually knowable before commit: invariant certification and runtime freshness (`Ready`, `Stale`, `RuntimeClosed`). Transaction-ID conflicts remain commit-time facts unless a concrete transaction identity is supplied. Candidate object queries support the same typed projection codecs as current snapshots, and `CandidatePreview` is an immutable presentation summary of source/target revisions, exact relation effects and diagnostics.

## History inverse status (Pass292–P293)

History is a projection of the durable causal revision-effect authority, not a second journal. A product `HistoryEntry` carries causal prerequisites, transaction identity, source/target revision and exact relation changes. `undo_plan()` returns an ordinary snapshot-bound Plan whose relation mutations are the exact swap of the committed delta and whose optional explicit model delta is the certified durable reverse complement. Candidate validation and durable commit remain unchanged. The entry must still be the live head; non-head rebase remains explicit future work. Undo is compensating history, so redo is the inverse of the committed undo entry.

For new mixed transitions the durable intent stores both the compact forward `DurableModelDelta` and its exact reverse complement. `kernel-plan` recomputes both directions from the source and target Revision during preparation and rejects a mismatched complement. WAL mutation codec v11 persists this information atomically with the same transaction authority; v10 mixed records remain backward-readable but cannot manufacture a missing complement. This makes lifecycle/carrier/reference-changing object history exactly invertible across reopen without a full old-Revision snapshot or fallback path.

## Historical snapshot status (Pass294)

A committed revision is a first-class immutable product world. `Database::at(revision)` reconstructs the exact logical Revision from the live durable head and the retained Γ-REIC causal effect graph; it does not create a second snapshot journal or persist full historical Revision copies. Reversible relation effects are traversable in both directions by swapping their exact deltas, while mixed effects additionally use the persisted P293 forward/reverse `DurableModelDelta` pair. If no exact reversible path exists, historical reconstruction fails closed.

The returned `ReadContext` shares the ordinary schema/query/object read vocabulary with a live snapshot but has no Plan capability. `db.history()` is therefore the history view anchored at current HEAD, while `db.at(r)?.history()` is the causal history anchored at revision `r`. Mutation from a historical world requires a future explicit rebase/restore operation rather than an implicit checkout.

## Non-head history undo / semantic rebase (Pass295)

Historical undo now works beyond the live head when the kernel can prove transport to the current revision. `HistoryEntry::undo_readiness()` reports whether an inverse is immediately ready, certified rebased, semantically conflicting, non-reversible, unavailable, or detached from its runtime. `undo_plan()` uses the same certificate and still returns an ordinary current-snapshot `Plan`.

The proof boundary is an exact semantic write footprint. Relation writes use Γ-canonical row-class coordinates, so two independent rows in the same relation can commute without relation-wide locking. Mixed object/model writes use carrier, field-owner, lifecycle and keeps-alive coordinates. Any overlap, opaque intervening transition, or unavailable exact history fails closed. There is no generic three-way merge, last-write-wins behavior, or fallback mutation path.

## Pass296 — exact revision-tagged watch foundation

Exact watch is now implemented in the Rust product runtime for queries accepted by the maintained relational differential program. A subscription is anchored to one live Revision, materializes the query's existing `MaterializedRelPlanState` once, and thereafter advances only through exact committed relation deltas recovered from the durable causal effect chain. Each delivered event names source/target revisions and exact inserted/removed query-result rows.

The runtime publication signal is a non-authoritative wake primitive only. Durable history plus the immutable Revision remain the source of truth after wake. No polling, result recomputation, state-diff fallback, Tokio dependency, or second event log is part of exact watch. Opaque/full/schema historical transitions and unavailable causal coverage fail explicitly. Cross-process wake transport remains open and must preserve the same exact causal-delta protocol.

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

Session authority propagates through derived product values. Restricted `ReadContext` values gate watch/history/write derivation; `Plan` carries its creating authority into Candidate; Candidate commit checks `Write`; Candidate reads check `Read`; `HistoryEntry::undo_plan()` checks `Write`. Plans from distinct session authorities cannot be composed. Therefore endpoint code cannot bypass authorization merely by switching from `SessionDatabase::commit` to `Candidate::commit`, object-query watch, or history inverse APIs.

The initial permission vocabulary is `Read`, `HistoricalRead`, `HistoryRead`, `Watch`, and `Write`. This is intentionally distinct from schema/semantic "capabilities" already present in kernel semantics. Transport, authentication mechanism and authorization policy remain future hosting/provider concerns; runtime enforcement consumes only the already-issued grants. Lean mechanization records the boundary law that deriving product values preserves the grant set and cannot manufacture missing write authority.


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
