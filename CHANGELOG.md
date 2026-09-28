## Pass305 — first-party local IPC transport conformance

- added separate `cfmd-transport-local` crate above `cfmd-host`;
- added a real Unix-domain-socket provider with private `0600` endpoint permissions and no implicit locality trust;
- added a bounded fixed authentication prelude whose declared evidence length is rejected before body allocation;
- passed authentication evidence unchanged into the configured `Authenticator`; the transport cannot mint principals or grants;
- added bounded per-connection request workers and bounded response backlog rather than unbounded thread-per-frame dispatch;
- kept a dedicated reader path so socket EOF closes `HostedConnection` and cancels blocked `NextWatch` without polling;
- added a small sequential local client helper for conformance/tooling use;
- added protocol wire error-frame encoding so transport admission/resource failures remain correlated by request id;
- deliberately limited P305 to Unix-domain sockets; Windows named-pipe support remains a separate provider implementation, not a change to database semantics.

## Pass304 — hosted security lifecycle / dynamic session authority

- replaced immutable copied session grants with one shared runtime authority cell carried by all derived product values;
- added monotone session revocation and host-driven permission refresh so already-created Plan/Candidate/History values observe current authority;
- added explicit bound/unbound channel-binding vocabulary visible to authentication providers without granting locality trust;
- added expiring authorization grants with `next_expiration` / `expire_due(now)` so an external event loop can schedule exact expiry without polling or a hidden host thread;
- added graceful server drain distinct from immediate close; draining rejects new connections while preserving already-admitted sessions;
- made grant removal cancel maintained watches when Watch authority disappears, and made connection close/revoke invalidate the underlying runtime session;
- added Lean session-lifecycle laws proving refresh/revoke preserve Revision and revoked authority cannot regain write through derivation.

## Pass302 — canonical hosted wire framing / negotiation

- added canonical transport-neutral binary framing with a fixed 20-byte header and request-id correlation;
- added wire framing version 1 plus explicit hosted-protocol/capability handshake;
- encoded all current hosted request/response/value/query/history/watch DTOs without exporting runtime/kernel types;
- added header-level payload admission and bounded collection/string/node/depth decoding before unbounded allocation;
- rejected unknown variants, non-canonical scalar discriminants, duplicate product fields and trailing bytes fail-closed;
- added `WireHostedSession`, which requires negotiation and dispatches only through the existing restricted `HostedSession`;
- deliberately added no socket/listener/TLS/IPC implementation.

# Changelog

## Pass303 — transport-neutral hosted server composition

- added separate `cfmd-host` crate above `cfmd-protocol`/`cfmd-runtime`;
- split authentication (`Evidence -> PrincipalId`) from authorization (`PrincipalId -> PermissionSet`);
- ensured concrete transports never receive unrestricted `Database` through the host API;
- added bounded active-connection admission and bounded per-connection in-flight frame execution;
- made disconnect/server shutdown close hosted sessions and cancel blocked watch consumers;
- explicitly rejected localhost/transport metadata as implicit trust or grant sources;
- added Lean hosted-boundary mechanization for authorizer-exact grants and disconnect non-authority;
- deliberately added no listener, socket, TLS, IPC, QUIC, daemon or background thread.

## Pass301 — exact hosted watch subscription protocol

- raised hosted protocol version to v2 and added protocol-owned `SubscriptionId`;
- added `OpenWatch`, blocking `NextWatch`, `WatchStatus`, `CancelWatch`, `CloseWatch`, and `CloseSession`;
- reused the exact P296/P298 `QueryWatch` authority instead of introducing protocol polling or an event log;
- returned exact initial results and revision-tagged inserted/removed result deltas, including empty result deltas on real revision advances;
- bounded maintained subscriptions per hosted session and fail-fast rejected concurrent consumers of one subscription;
- made cancellation/session close wake already-blocked consumers without requiring transport-specific wake logic;
- extended Lean notification mechanization so hosted protocol watch lifecycle cannot manufacture authoritative revisions.

## Pass300 — transport-neutral hosted protocol/session foundation

- added separate `cfmd-protocol` crate above `cfmd-runtime`;
- added protocol-owned value/row/query/history/mutation/request/response DTOs;
- bound `HostedSession` exclusively to restricted `SessionDatabase`;
- added current/historical query, anchored history and relation commit operations;
- required explicit commit base revision and transaction identity with stale-base fail-closed behavior;
- added protocol complexity limits before runtime execution;
- sanitized internal/recovery/invariant diagnostics at the untrusted boundary;
- deliberately added no listener, TCP/IPC, codec or authentication implementation.

## Pass299 — hosted session/security authority boundary

- added `PrincipalId`, `Session`, `Permission`, `PermissionSet` and `SessionDatabase`;
- kept authentication/network transport outside `cfmd-runtime`: the trusted host issues a session, runtime enforces it;
- propagated authority through `ReadContext -> Plan -> Candidate` and durable history entries;
- closed bypasses through Candidate commit/read, object-query watch and history undo derivation;
- rejected Plan composition across distinct session authorities;
- added `ErrorKind::PermissionDenied`;
- mechanized grant-preservation/no-write-escalation laws in Lean 4.34.0.

## Pass284 — object-first Rust domain model

- Added object-first Rust schema/query/write mapping over `cfmd-runtime`.
- Object operations return proposed `Plan` values; Plans are database/snapshot-bound and composable.
- Kept relation-first IR as the universal binding/tooling substrate.
- Deferred deep reference traversal until explicit identity/cardinality authority exists.

# Changelog

## Pass298 — deterministic watch lifecycle and bounded catch-up

- added cloneable `WatchCancellation`, `QueryWatch::close()` and `ErrorKind::WatchClosed`;
- made blocking `recv()` cancellable without timeout polling;
- made a blocked watch stop deterministically when the last runtime owner closes;
- added `WatchStatus::{Current,Lagging,Cancelled,RuntimeClosed,Unavailable}` with exact causal lag count;
- kept backlog out of RAM: one maintained query state + one anchor, one durable transition consumed per receive;
- changed the provider primitive to explicit wake-only `notify_waiters()`, with publication as a wrapper after durable state advance;
- mechanized cancellation/shutdown non-authority laws in Lean 4.34.0.

## Pass294 — exact historical worlds (`db.at`)

- added `Database::at(revision)` as an immutable historical product world;
- reused the ordinary relation/object query vocabulary for historical reads;
- added revision-anchored `ReadContext::history()`, making `db.at(r)?.history()` a causal history view at `r`;
- reconstructed historical revisions from the current head plus exact reversible causal effects rather than persisting full per-revision snapshots;
- traversed the reversible causal graph in either direction, including P293 mixed forward/reverse model deltas;
- removed write capability from historical read contexts and failed closed across unavailable/non-reversible history boundaries;
- added reopen, anchored-history, mixed lifecycle and historical-read-only E2E coverage.

## Pass293 — exact mixed history complement

- persisted an exact compact reverse `DurableModelDelta` for newly committed mixed transitions;
- added WAL mutation codec v11 while retaining v10 mixed-record decoding;
- made lifecycle/carrier/reference-changing history entries exact `Plan` inverses after reopen;
- kept undo/redo on the ordinary `Plan -> Candidate -> commit` product pipeline;
- added E2E coverage for entity lifecycle undo/redo and strong-reference field inversion;
- rejected complement mismatches during mixed revision preparation instead of trusting callers.

## Pass283 — typed Rust product DX foundation

- reconciled the latest Ubuntu namespace CI fixes from GitHub;
- added typed Rust relation/field handles over `cfmd-runtime`;
- added schema-checked scalar codecs, typed equality predicates and tuple projections;
- added typed row insert/remove helpers through extensible `RowCodec`;
- added typed `all/first_or_none/one/one_or_none` terminals;
- kept deep relationship traversal pending an explicit reference/cardinality contract rather than treating arbitrary joins as object navigation.

## Pass282 — Rust product create/schema boundary

- reconciled the GitHub CI fix that narrowed crash-dump ignore from `core.*` to `core.[0-9]*`, preventing Rust `core.rs` source files from being silently omitted;
- added facade-owned recursive types, primitive semantic module presets and relation schema construction;
- added `Database::create` and typed-empty durable relation bootstrap without sentinel rows;
- added schema introspection through `ReadContext`;
- changed the E2E product test so it creates, writes, closes, reopens and queries a database without importing any `kernel-*` crate.

## Pass281 — Rust product runtime foundation

- added `cfmd-runtime`, a stable product/runtime anti-corruption layer above the frozen kernel graph;
- added facade-owned identifiers, recursive values, query IR, prepared query execution, immutable read contexts and durable relation Plans;
- added an end-to-end durable regression that opens, queries, commits and re-queries only through `cfmd-runtime`;
- changed product sequencing to Rust-runtime first: future Python/.NET/Studio surfaces bind to `cfmd-runtime` rather than calling kernel crates directly.


## Unreleased

### Global kernel closeout (Pass280)

- completed/froze the global kernel hostile/refactor campaign for the current declared scope;
- revalidated the historically heavy query/plan/semantics/durability line against the final workspace;
- completed final model/schema ownership cleanup and bidirectional subtype-closure authority;
- retained an evidence-driven reopen policy rather than continuing cleanup by inertia.

### Repository / CI release preparation

- reconciled GitHub-side workflow fixes so shell gates run through explicit `bash` and do not depend on executable-bit preservation;
- restored Cargo-vendor `.gitignore` exceptions for `Cargo.toml.orig` and `.cargo-checksum.json`;
- strengthened repository verification to catch missing literal Rust `include!("...")` targets and incomplete/corrupt Cargo vendor snapshots before compilation;
- made `ci-rust.sh` run repository-integrity verification before Rust gates;
- refreshed current README/spec/status/architecture/report documentation;
- added a current kernel hostile/freeze ledger.

### Product roadmap

- changed product sequencing from “standalone Rust application facade first, Python later” to a Python-first application facade backed by a stable Rust runtime/facade boundary;
- added the Python facade design source and product roadmap covering deep symbolic queries, exact watch, Plan/Candidate/history, local tooling/Studio and later language surfaces.

### Repository projectization (Pass121)

- reorganized pass-oriented research snapshot into a conventional project repository;
- preserved historical reports/evidence under `docs/history/` and `artifacts/`;
- recorded 22/22 historical problems closed for the declared support scope;
- added Rust/Lean CI policy and toolchain pins.

## Pass285 — object identity and strong references
- Added typed `Id<T>` / strong `Ref<T>` entity model and `cfmd_entity!`.
- Added atomic final-state identity/reference validation for object-generated plans.
- Added nested deep reference predicates lowering to existing JoinEq + root semantic distinct.
- Preserved value-object `cfmd_object!` behavior for objects without identity.

## Pass286 — object cardinality foundation

- Added typed optional strong references (`Option<Ref<T>>`) with structural option equivalence.
- Added symbolic reverse-many relationships that do not appear on materialized Rust objects.
- Added explicit `any`, vacuous `all`, `none`, and `count().eq(n)` query semantics over reverse-many paths.
- Cardinality lowering reuses universal `JoinEq`, `AntiJoin`, `Difference`, and `Group(Count)` relational operators; no cardinality-specific kernel query path was added.
- Open backend payer: migrate strong object references to persisted kernel lifecycle/carrier authority so incoming-reference integrity is global across reopen and target deletion.

## Pass288 — 2026-09-28

- added first-class mixed revision transitions in `kernel-plan`;
- entity/object commits now incrementally update physical relations/materializations instead of rebuilding the full physical store;
- preserved exact full-target durability/recovery identity;
- added incremental-publication + reopen + idempotent-retry regression coverage.

## Pass295 — certified non-head history undo/rebase

- Added kernel-owned semantic write footprints for durable history effects: Γ-canonical relation classes plus carrier/field/lifecycle/keeps-alive coordinates.
- Added exact non-head inverse rebase certification across intervening durable causal effects.
- `HistoryEntry::undo_readiness()` now reports ready/rebased/conflict/runtime/non-reversible/unavailable states.
- `HistoryEntry::undo_plan()` can return a current-HEAD Plan for an older entry when the kernel proves footprint disjointness; overlaps and opaque effects fail closed as `HistoryRebaseConflict`.
- Added regressions for durable reopen rebase, same-relation disjoint classes, same-class conflict, and independent object/entity coordinates.

## Pass296 — exact watch/reactivity foundation

- Added `QueryWatch`, `ObjectWatch`, and typed `ProjectionWatch` product surfaces with `initial()`, non-blocking `try_recv()`, and blocking `recv()`.
- Added revision-tagged exact watch events carrying inserted/removed query-result deltas.
- Reused `kernel-query::MaterializedRelPlanState` and its compiled differential ExecGraph; watch does not recompute whole query results after commits.
- Added a wake-only `DurableRuntime` Revision publication generation/`Condvar`; durable causal history remains state authority.
- Added fail-closed watch boundaries for historical snapshots, unavailable causal coverage, and opaque/full/schema transitions.
- Kept the runtime dependency-free with respect to Tokio/async runtimes; async adapters remain a future integration layer.

## Pass297 — publication notification / hosting boundary foundation

- Replaced the hard-wired kernel `Condvar` publication signal with the `RuntimeRevisionPublicationNotifier` wake-only provider contract.
- Added facade-owned `PublicationNotifier` and first-party `InProcessPublicationNotifier` without exposing kernel types.
- Added `Database::create_with_publication_notifier` / `open_with_publication_notifier` for host/event-loop/transport integration.
- Kept notifications outside mutation and multi-writer correctness authority: spurious wakes cannot fabricate `WatchEvent` state, and semantic rebase/conflict certification remains kernel-owned.
- Added Lean mechanization proving that arbitrary duplicate/spurious notifications preserve authoritative Revision state.
- Deliberately did not start an IPC server, bind a socket, trust localhost, or add a networking dependency.

## Pass306 — single-file physical container foundation

- Added an append-only `SingleFileContainer` physical-storage foundation for future `*.cfmd` databases.
- A complete generation is streamed and `sync_all`'d before authority publication through alternating 4 KiB root slots.
- Root slots and generation/section payloads are SHA-256 authenticated; section descriptors are range/ordering/uniqueness checked and independently verifiable.
- Recovery ignores unpublished orphan generations and torn replacement slots, but a checksum-valid newest root whose generation is corrupt fails closed and never rolls back to an older acknowledged generation.
- Valid dual roots must form one consecutive sequence/generation/parent-digest chain.
- Added streaming section extraction and avoided whole-generation staging buffers. No temp/sidecar recovery authority is part of the format.
- Added Lean `CFMD/SingleFile.lean` for the root-publication/no-silent-rollback law.
- This pass establishes the physical container only; the live `DurableRevisionStore` WAL/checkpoint pipeline is not yet switched to `Database::open("*.cfmd")`.

## Pass307 — single-file live WAL authority

- Embedded the existing `FileRevisionWal` semantics into the `SingleFileContainer` as an offset-bounded journal region; prepare/commit frame codecs, recovery scanner, LSN continuity and idempotency semantics are reused rather than reimplemented.
- Added root-authoritative journal state: `journal_offset`, open/sealed `journal_end`, `journal_first_lsn`, and exact sealed `journal_next_lsn`.
- Generation rotation now seals and synchronizes the current journal before appending a new generation; the new root carries the exact next WAL LSN.
- Recovery of a crash after journal seal but before generation root publication validates the sealed WAL prefix, truncates only unpublished orphan bytes, republishes the old journal as open, and continues at the same LSN sequence.
- Torn live-WAL tails are truncated only to the existing WAL scanner's last-good frame boundary inside the same `*.cfmd` file.
- Added exhaustive byte-prefix WAL recovery coverage plus WAL/generation rotation and sealed-orphan crash regressions. No temp/sidecar recovery authority was introduced.

## Pass308 — live single-file DurableRevisionStore integration

- Added `DurableRevisionStore::create_single_file` / `open_single_file` over the P306/P307 container and live WAL.
- Reused the existing prepare/commit/idempotency/causal authority state machine rather than introducing a second store implementation.
- Added synchronous single-file checkpoint rotation carrying checkpoint, metadata, prepared-cut capsule and exact next WAL LSN in one self-contained file.
- Added live create/commit/reopen/rotate/commit/reopen regression proving no store sidecars are created.
- Replication mutation, streaming checkpoint, external freshness and physical compaction remain explicit fail-closed payers rather than hidden directory fallbacks.

## Pass309 — single-file replication authority lane

- Embedded replication-authority records into the same single-file WAL LSN stream as transaction durability using a distinct auxiliary WAL record kind; no replication sidecar remains necessary for live single-file authority.
- Single-file replication mutations now flush through the active WAL durability barrier before returning and are replayed from WAL recovery without advancing the logical database head.
- Checkpoint rotation archives accumulated replication frames into the generation `ReplicationAuthority` section and resets the live replication lane while preserving exact replay across reopen.
- Added regression proving replication membership survives live WAL reopen and synchronous generation rotation with exactly one `*.cfmd` file.
- Kept pinned-cut streaming checkpoint fail-closed in single-file mode: a correct implementation requires a certified overlapping-WAL generation transition rather than weakening the P306/P307 root-chain LSN law.

## Pass310 — durability backend separation

- Replaced `DurableRevisionStore { directory, lock, Option<SingleFileContainer> }` physical state with one explicit `DurabilityBackend` owner.
- Centralized directory/single-file physical ownership and capability declarations while leaving transaction, revision, retry/idempotency and causal semantics common.
- Delegated captured replication-authority frame persistence to the backend instead of branching on single-file layout in the replication facade.
- Added backend capability checks for streaming checkpoint, external freshness and physical compaction; unsupported external freshness is now rejected before any runtime authority mutation.
- Added Lean backend-separation laws and a regression for non-mutating capability rejection.

## Pass 311 — certified single-file streaming carry-forward
- Single-file streaming checkpoints now preserve a pinned checkpoint cut while ordinary commits and replication authority continue in the active WAL.
- Finalization seals the old journal, appends the unpublished generation, copies the exact post-cut WAL suffix, rescans it against the pinned revision/prepared seeds, and publishes the new root only after the suffix certifies the current durable endpoint.
- Root-chain validation distinguishes ordinary contiguous rotation from certified overlapping carry-forward; arbitrary overlap remains invalid.
- Active single-file WAL handles now use independent file descriptions rather than `File::try_clone()`, preventing generation reads from moving the WAL append cursor on platforms with shared cloned seek offsets.

## Pass312 — single-file freshness and physical-compaction parity

- Replaced directory-specific external-freshness probing with backend-neutral generation/WAL freshness material.
- Added freshness-aware single-file open/recovery across live WAL, synchronous rotation, and P311 streaming carry-forward; ordinary open of an externally anchored store fails closed.
- Added single-file rollback/WAL-truncation freshness regressions and verified that compaction preserves the same freshness digest/binding.
- Implemented crash-safe in-file physical compaction by copying the current generation plus sealed WAL, fully validating the relocated authority, publishing a certified same-generation relocation root, reopening the live journal, and only then truncating obsolete tail bytes.
- Compaction preserves logical generation, Revision, generation/WAL digests, prepared/replication state and freshness material; no temp file or sidecar authority is introduced.
- Moved compaction dispatch behind `DurabilityBackend`, completing the parity capability surface without layout branching in semantic durability code.



## Pass313 — unified DatabaseBuilder / single-file product default

- Added `DatabaseBuilder` and `Storage::{Auto, SingleFile, Directory}` as the canonical Rust product construction model.
- New paths resolve to the parity-complete single-file backend by default; existing files/directories are reopened according to their physical shape, without extension-based authority.
- Routed `kernel-plan::DurableRuntime` create/open through an explicit durability backend while preserving one runtime/recovery protocol.
- Brought single-file bootstrap artifact-core handling to parity with directory bootstrap.
- Kept publication notification as a database-runtime builder option and hosting/authentication outside database file creation.
- Added `DatabaseHostingExt::{host, host_with_limits}` in `cfmd-host` for `db.host(...)` composition without a runtime→host dependency cycle.
