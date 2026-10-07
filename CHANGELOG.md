- PASS595: replaced the five-traversal replication-authority frame-source publication path with a stream-composable CFAS v2 identity and exactly two traversals (plan + write/proof), added source-drift rejection and unpublished-tail rollback, and scheduled PASS596 as the branch-level hostile/Git-ready consolidation checkpoint.
- PASS591: mandatory persistence-transition hostile cleanup removed the redundant runtime persistence wrapper, fixed loss of disk-only retained historical epochs during durable -> volatile backend retirement by materializing the exact retained closure first, corrected the demotion complexity law, and selected canonical semantic replication-authority transfer as the next payer.
- PASS588: hostile replication-authority cutover audit confirmed that production checkpoint/recovery/compaction already uses the immutable `CFAS/CFAO + CFLN` closure; removed the dead generation `ReplicationAuthority` section discriminator from the active SingleFile grammar, refreshed current architecture/spec wording, and strengthened rotation coverage around an empty authority delta.
- PASS587: hostile Rust productization audit restored the full kernel-plan acceptance gate after PASS581 encoded semantic lanes invalidated stale physical-owner assertions; QCN exact-delta state remains directly consumable, shared encoded lanes own normal execution where admitted, strict all-target kernel-plan Clippy is clean without suppressions, and no new engine/fallback was introduced.
- PASS581: integrated catalog-free SAMF over one shared revision/store semantic-class catalog and encoded relation lanes; removed SAMF-local product class authority, preserved canonical durable tuples, fixed same-revision equivalence-drift lane reuse and replacement-time class release, and kept retention-profile synthesis paused.
- PASS578: exposed the proven Memory -> Durable theorem as `Database::memory::<S>()` / `persist`, preserved live Context/clone identity across same-revision promotion, moved persistence-location truth into the runtime owner, enforced `PersistenceTransition` before provider/staging side effects, and kept Durable -> Volatile absent.
## PASS568 — pre-release format normalization + durable compatibility removal

- PASS577: exposed certified sealed external-freshness authority transfer through the product boundary. Freshness-aware reopen reuses the ordinary recovery path; transfer requires exclusive runtime ownership, mutates the same Database handle into the target, enforces Schema-owned `AuthorityTransfer` before restricted side effects, and projects failures through structured recovery diagnostics. FORMAT V1 is unchanged.
- Corrected PASS557 policy: CFMD has not shipped a compatibility release. `FORMAT_VERSION = 1` remains the selected release-candidate discriminator, but pre-release snapshots carry no backward-compatibility promise.
- Reset pass-era current-only checkpoint `7 -> 1`, physical-realization `4 -> 1`, and CanonicalEqKey/semantic-index `2 -> 1`; old decoder ladders were already absent, so these numbers carried no semantic compatibility value.
- Removed production arbitrary-target relation commit (`commit_revision_durable_full_exact` and durable/supervisor `commit_revision` edge). Current relation durability derives its endpoint from source + exact deltas.
- Moved several compatibility conveniences that were used only by regressions behind `cfg(test)` and renamed misleading legacy terminology in active query/advisor code.

## PASS567 — kernel version/compatibility re-inventory

- Added `docs/status/KERNEL_VERSION_COMPAT_INVENTORY.md` and a project rule requiring every future `vN`/legacy/compat discovery to be classified in-repository during the discovering PASS.
- Re-audited active `kernel-*` version tags after FORMAT V1: checkpoint codec 7, realization codec 4 and CanonicalEqKey encoding 2 are current-only/fail-closed contracts, not live decoder stacks.
- Confirmed query legacy-delta/BFC adapters are test-only oracles and the PASS472 `LegacyIndex` execution family remains absent.
- Identified two real follow-ups: generic `commit_revision_durable_full_exact` compatibility commit, and advisor-owned statistics/quotient-factor families that are still current cyclic-optimizer capabilities rather than removable dead code.
- Replaced the sealed-freshness checkpoint positional argument bundle with `SingleFileCheckpointAuthority`; full kernel-durability Clippy is warning-free without a new suppression.
- Full Clippy then found and closed a kernel-plan volatile-owner `map(...).unwrap_or(false)` hygiene debt via `Result::is_ok_and`.

## PASS558 — public history-retention contract

- Exposed existing migration historical anchors as `HistoryRetentionPin` with explicit `SchemaMigration` reason; no new persisted retention table and no FORMAT V1 byte change.
- Added irreversible/idempotent `Database::release_history_retention`, preserving causal migration, retry and replication authority while retiring only exact source-epoch materialization.
- Fixed volatile release split-brain: runtime retained epoch retirement now follows canonical anchor existence rather than the presence of a durable checkpoint receipt.
- Kept watch/read/GC semantics exact and fail-closed: no reconstruct-on-release or reader-reference-count fallback.

## PASS557 — first released durable FORMAT_VERSION V1

- Froze `kernel_durability::FORMAT_VERSION = 1` as the first public on-disk compatibility boundary after re-auditing Schema.Access, enum/sum and Memory -> Durable pre-format ledgers.
- Unified single-file and directory outer durable envelopes on released V1; earlier PASS-era `3/2/1` component tags are no longer compatibility ancestors.
- Removed unreleased checkpoint semantic codec V1-V6 decoder branches; released V1 accepts the current codec V7 only, including nested migration/historical semantic contexts.
- Added an explicit public read/write compatibility matrix, no released upgrade sources, no downgrade targets, and fail-closed unknown/future single-file format admission.
- Documented current persisted subcodecs as part of the V1 byte-language obligation and selected staged reopen-verified authority transfer as the only future format-upgrade law.

## PASS544 — structured migration/bridge diagnostics
- PASS545: closed migration observe/cut-over as an indexed projection of authoritative causal history; no migration progress state machine.

- Added a single runtime-owned structured migration diagnostic algebra with domain, reason and exact semantic coordinates; frontends no longer need to parse kernel `TransportError` prose.
- Attached exact diagnostics to migration validation/preview transport failures and current-world read/write/lifecycle bridge failures.
- Preserved `ErrorKind` as the coarse control-flow category while exposing optional structured migration detail through `Error::migration_diagnostic()` and the public `cfmd::Diagnostic`.
- Kept the ubiquitous runtime `Error` compact by storing migration diagnostics indirectly; strict Clippy caught and rejected the first inline representation because it inflated every `Result<_, Error>`.
- Added stable Access/preparation diagnostic domains for the selected Schema-Owned Access and workflow lines without implementing a second policy/migration evaluator.

## PASS540 — bridged remote activation + current-world authorization

- Added hosted `ContractHead { schema_revision }` query activation: old-language remote reads compile through retained verified `CurrentSchemaBridge` and authorize only against current-schema footprints.
- Hardened explicit old typed contracts so `#[cfmd(schema_revision = A)]` with `A != current` always resolves through verified bridge rather than attempting shape-compatible direct bind first.
- Added restricted-Session bridged field-patch regression proving source-world grants cannot authorize current-target publication.
- Imported three R&D source notes into `docs/rnd/` and added abstract long-horizon authorization, semantic-kernel and embedded-DX directions to the productization ledger.
- Kept bridged create/delete/relationship/precondition transport fail-closed pending exact object/lifecycle/relationship coordinate laws; no generic fallback was added.

## PASS539 — bridged typed Context activation

- Added admission-scoped `CurrentSchemaBridge` activation for explicitly-versioned old `CfmdSchema` contracts; old queries compile to current-world IR before authorization/execution.
- Added `#[cfmd(schema_revision = N)]` and exact relation/model-field identity transport for bridged field-patch lowering.
- Kept create/delete/relationship/precondition compatibility fail-closed until separate exact lifecycle/coordinate laws exist; no historical-schema or generic fallback was added.

## PASS523 — Γ-keyed grouped exact-measure generalization

- Generalized sparse grouped validation buckets from count-specific state to one exact-measure witness while preserving separate live membership authority.
- Added grouped selected `ExactF64Sum` with Γ-canonical keys, delta-local maintenance, typed object-first DX and durable reopen.
- Kept cross-relation grouped comparison fail-closed pending an explicit group-domain alignment theorem; no query/HAVING fallback or implicit missing-group semantics were introduced.

## PASS379 — stable field identity across Rust renames

## 2026-10-04 — PASS507 GitHub checkpoint sync

- Refreshed repository-facing docs/spec/status to the PASS507 product boundary.
- Synchronized Rust CI prerequisites from GitHub (`ripgrep`, Ubuntu user namespaces) without importing the older PASS472 implementation snapshot.
- Updated the Lean refinement checker to follow split Rust modules/includes and current streaming-checkpoint durability cuts.
- Added Git-index-aware manifest generation/verification for GitHub CI while preserving archive SHA verification.
- Normalized workspace Rust formatting with pinned Rust 1.98.1.
- Closed the remaining strict Rust CI debt on PASS507: `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` and the complete `scripts/ci-rust.sh` gate now pass.
- Updated the stale derive compile-fail fixture to the schema-neutral `Database::builder(...).create_authoritative::<S>()` creation law.
- Refreshed `SurfaceKernel.lean` and its source-refinement mirror for the production `FilterOrderConst` and `Union` relational constructors; both Python refinement binders pass.

- `#[cfmd(rename_from = "old_name")]` now separates the current Rust field name from the durable semantic field name used by CFMD equivalence/order/reference coordinates.
- Partial Context projection, exact object binding, scalar patches, reference contracts and field orderings all resolve through the durable semantic name rather than the current identifier.
- Old same-key readers using the previous field name can therefore read and patch a database created from the renamed authoritative entity; persisted Semantic Rules continue to govern the same coordinate.
- Derive rejects duplicate durable field identities and invalid/redundant `rename_from` declarations. Field names remain the default semantic identity when no rename metadata is present.

## PASS375 — entity-owned rule DX + schema composition R&D

- `#[derive(CfmdEntity)]` now carries database-owned scalar rules directly on Rust fields through `#[cfmd(range(...))]`, `#[cfmd(length(...))]`, and `#[cfmd(one_of(...))]`.
- Removed `SchemaBuilder::object_field_rule`; object rules no longer attach later through a string field name. Generated entity metadata installs them on the actual object-relation columns during schema assembly.
- Rule attributes are deterministic schema literals rather than arbitrary Rust expressions, and incompatible field/rule combinations fail at derive time.
- Schema-composition DX was reviewed separately from implementation; the preferred direction is an explicit user-selected schema root whose named `EntitySet<T>` fields are both membership declarations and the eventual typed collection facade. No global/autodiscovery registry is planned.

## PASS374 — database-owned semantic field rules foundation

- Added schema-authoritative typed value rules for `I64Range`, `TextLength`, and finite `TextOneOf` membership; rules attach to kernel fields or relation columns rather than living in application callbacks.
- P374's temporary object-rule attachment API was superseded and removed by P375; entity derive metadata now maps field rules directly to their actual relation-column authority.
- Candidate/commit validation rejects violating states through the existing revision validation boundary; dynamic VMF also exposes exact rule witnesses.
- Checkpoint codec v3 persists field and relation-column rules; hostile public coverage proves a rule still rejects invalid candidates after reopen.
- Regex remains OPEN until CFMD owns a deterministic pattern engine/module contract; no ad-hoc parser or host regex callback was introduced.

## PASS373 — mixed/object certified residual commit

- Extended dual durable client-intent vs realized-effect authority from relation-only commits to mixed/object transitions.
- Certified stale object commits now rebuild from current HEAD, Γ-residualize relation effects, apply the certified model delta, and derive exact realized model delta/complement for history and recovery.
- Durable `MixedRevisionResidualExact` keeps original relation/model intent stable for retries while WAL/history/recovery retain the realized transition.
- Hostile regression covers independent first object inserts sharing carrier creation, retry idempotency, exact undo of only the residual realization, and reopen.

## PASS372 — stable client intent + realized residual commit

- Added durable `RelationDataResidualExact`, separating canonical client retry identity from the exact residual relation effect realized against a certified newer HEAD.
- WAL/metadata/recovery retain both mutation sets; history consumes realized mutations while idempotency compares client mutations.
- `Database::commit(&tx)` now residualizes certified pure relation-data Set effects by Γ-class and publishes even an empty residual as a durable no-op revision.
- Hostile regression proves concurrent identical relationship attach, retry idempotency, single-edge state, and persistence reopen.

## PASS369 — native Γ-aware Union and predicate disjunction

- Added first-class relational `Union` across exact evaluation, maintained differential execution, execgraph state, transport, durable query encoding, and physical planning.
- Set union is Γ-support union: rows selected by both branches remain one semantic row and are removed only when the final branch support disappears. Bag union is additive multiset union.
- `ObjectPredicate::or(...)` now lowers directly to one native Union node instead of a De Morgan/Difference expansion or host callback.
- Exact, Candidate and maintained-watch regression covers overlapping branches, matching insertions and irrelevant changes.

## PASS368 — relationship cardinality predicates preserve zero-degree semantics

- Replaced the facade-only `ManyCountEq` leaf with one `ManyCountPredicate` law while keeping `children.count().eq(n)` source syntax unchanged.
- Relationship counts now support `ne`, `greater_than`, `greater_than_or_equal`, `less_than`, `less_than_or_equal`, and inclusive `between` through kernel Group/order/join/anti-join algebra.
- Zero-degree owners are preserved mathematically rather than synthesized as fake aggregate rows: predicates rejecting zero join matching positive Group rows; predicates accepting zero anti-join only the positive groups that violate the predicate.
- Exact, Candidate and maintained-watch regressions cover the zero-to-one transition without host-side maps, materialized relationship targets, or SQL-shaped fallback logic.

## PASS367 — column equality + boolean negation + wide typed query shapes

- `Field::eq(...)` now accepts either an ordinary value or another field of the same typed domain; field-to-field equality lowers directly to maintained `FilterEqColumns`.
- `Field::ne(...)` and `ObjectPredicate::not()` use the existing maintained `Difference` algebra; no callback predicate or host-side filtering was introduced.
- Typed `GroupKey` and projection tuples now extend beyond the former 3-column facade ceiling, with tuple support through arity 12 and const-generic homogeneous array keys/projections.
- Exact, Candidate and watch regressions cover field equality; public tests also instantiate 4-column group/projection shapes and array projection.


## PASS359 checkpoint 1 — transaction-owned relationship mutation DX

- `Many<T>`, `OwnedMany<T>` and filtered relationship selections now write directly into `&mut Transaction`; ordinary relationship mutation no longer returns a user-visible `Plan`.
- Advanced exact-plan construction remains available under explicit `*_plan` method names for tooling/protocol code.
- Relationship mutations rebind evaluation to the transaction's single formation world, so one transaction cannot silently compose edge operations evaluated at different revisions.
- Filtered selections are re-evaluated against that formation world without materializing target Rust objects merely to obtain identities.
- Owned relationship mutation keeps exclusivity, orphan policy and derived preview effects on the same existing Plan/Candidate kernel path.

## PASS358 — Passive transaction + database-owned CRUD DX

- Ordinary writes now start with `Transaction::new()`; no database, revision, `Plan`, or `TransactionId` is required at construction.
- `Database::objects::<T>()` / `SessionDatabase::objects::<T>()` expose current object collections directly.
- `ObjectSet::add/remove` and `ObjectQuery::update/delete` write explicitly into `&mut Transaction`; plan construction remains internal.
- `Transaction::from(snapshot)` is the explicit strict-snapshot mode and refuses silent transport after HEAD changes.
- Adaptive transaction IDs are generated lazily from the operating-system CSPRNG on first exact mutation.
- Deterministic transaction IDs remain a hidden protocol/testing escape hatch; `commit_plan` remains the explicit low-level publication path.

## PASS357 — stable semantic transaction identity and automatic certified commit

- separated client semantic intent identity from one concrete source/target durable realization without adding a second retry ledger or hash authority;
- made relation/mixed durable intents compare canonical forward effect and pinned semantic authority for retry identity while retaining exact realization bytes for recovery;
- enabled `Database::commit(&tx)` to certify and publish an older transaction on current HEAD when kernel-change/runtime history proves transport safe;
- preserved fail-closed conflicts and made retry after transported commit return `AlreadyCommitted` with the actual published revision;
- added durability regression proving revision-recertified intents remain the same client intent while different payloads do not.

## PASS355 — semantic transaction readiness and merge-kernel bridge

- Connected product transaction readiness/preview to Γ-canonical causal conflict certification.
- Added runtime prospective transition rebase certificates and product `TransactionReadiness`.
- Kept durable commit revision-pinned until transport-stable semantic intent identity exists.
- Identified durable action-law loss (`EnsurePresent` collapsing to coordinate overlap) as the next merge-productization blocker.

- P353: replaced abbreviated public ordered-predicate methods with `greater_than/greater_than_or_equal/less_than/less_than_or_equal`, removed the pre-release aliases, and mechanically split P352 ordered-query dispatch paths to restore strict Clippy without semantic changes.
## Pass350 — Python hostile/product validation
- P352: added Γ-native ordered/range predicates (`greater_than/greater_than_or_equal/less_than/less_than_or_equal/between`) with exact maintained-watch deltas and stable per-field ordering identities.

- moved the Python validation harness onto the public `cfmd` facade rather than a direct `cfmd-runtime` dependency;
- closed a public-facade leak by exporting low-level `QueryWatch` / `WatchEvent` through `cfmd::dynamic`, with compile-contract coverage;
- expanded CPython 3.13 black-box coverage across typed CRUD/query/update/delete, deep `Ref` traversal, `OwnedMany` move/orphan preview, history undo, persistence reopen, typed object watches, cancellation, GC churn, 512 relevant watches and 256 unrelated watches;
- found an asyncio loop-shutdown lifetime hole: a bare PyO3 Future is not part of `asyncio.all_tasks()` and can survive loop close while retaining an in-flight Rust receive; the validation bridge now taskifies each receive so loop shutdown cancels it and cancellation reaches the Rust wait registration;
- retained P349 replay reservation so cancellation/commit races remain lossless; no second CFMD event authority or Python-side recompute path was introduced.

## Pass349 — Python asyncio proof

- proved CPython 3.13 `await watch.next()` and `async for` over the executor-neutral Rust watch law through PyO3/maturin;
- reproduced and closed the Python cancellation/ready race with replayable delivery reservation at the FFI boundary;
- kept Python as a validation consumer rather than a semantic authority or second watch implementation.

## Pass348 — direct watch async + dependency frontier

- removed the transitional `into_async()`/wrapper layer: sync, nonblocking and async consumption now live on one watch object;
- indexed pending readiness by exact maintained-query relation dependencies, avoiding executor wakes for unrelated relation publication;
- quotiented output-equivalent causal revisions out of the public event stream while retaining exact durable causal catch-up.

## Pass347 — hostile async notification closure

- moved executor Waker registration from the kernel bridge sidecar into `PublicationNotifier`, unifying direct host wake signals with blocking and async waiters;
- added executor-task migration/Waker replacement, cancellation, repeated-spurious-wake and 2,000-pending-watch hostile coverage;
- exposed `AsyncWatch::try_recv` and bounded `AsyncWatch::drain_ready` so executor integrations can slice durable catch-up without reaching through the wrapped watch;
- identified the next scaling target as dependency-frontier wake filtering: O(K) delivery to K relevant subscribers is unavoidable, while unrelated subscriptions sharing one runtime should remain asleep.

## Pass305 — first-party local IPC transport conformance

## Pass339 — generated reverse-many/cardinality SDK

- `CfmdEntity` supports symbolic reverse-many declarations without collection fields in materialized structs.
- Public relationship metadata distinguishes required-one, optional-one and many cardinality.
- Reverse-many `via` type/target and target registration now fail during schema construction.
- Expanded derive compile-fail diagnostics; async remains deferred to the multiplexed adapter design.

## Pass337 — public Rust application facade

- added the public `cfmd` crate as the only direct dependency required by ordinary Rust applications;
- kept object-first application vocabulary at the root/prelude and moved the relation-first protocol behind explicit `cfmd::dynamic`;
- added facade-owned diagnostic codes and a public-surface CI gate forbidding direct `kernel-*` dependencies;
- added end-to-end `cfmd`-only product tests and a TODO example;
- intentionally deferred Tokio/async watch to an adapter over the existing exact-watch semantics.


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

## Pass354 — database-owned transaction control surface

- Superseded self-publishing `Transaction::preview/commit` with database-owned `Database::preview(&tx)` and `Database::commit(&tx)`.
- Replaced `Transaction::apply(plan)` with `Transaction::add(plan)` to make composition read as mutation of the transaction container rather than mutation of the database.
- Removed public `Candidate::commit`; Candidate is inspectable proposed state only.
- Renamed the advanced raw-plan publication path to `Database::commit_plan` / `SessionDatabase::commit_plan`.
- Added database-identity and session-authority checks at preview/publication control points.
- Removed `Plan` from the ordinary `cfmd::prelude`; low-level plan construction remains available explicitly.
- Added `docs/api/CFMD_DX_CONTROL_MODEL_RU.md` as the control-surface law for subsequent DX refactors.

## Pass356

- carried kernel-change `RewriteActionLaw` through runtime causal footprints for forward transaction transport;
- added a coordinate-domain-independent action-map classifier in kernel-change;
- set relation deltas now expose exact presence laws while bag deltas remain coordination-required until multiplicity laws are represented;
- model carrier-presence and lifecycle facts retain presence laws, while entity carrier membership remains conservative to prevent same-identity/different-payload false merges;
- transaction conflicts now distinguish definite conflict effects from exact effects that still require coordination;
- added hostile regressions for independent first-object inserts and preserved same-identity conflict behavior.

## Pass360 — unified history mutation DX + ordered hostile closure

- Added database-owned `undo(&mut Transaction, &HistoryEntry)` and `undo_latest(&mut Transaction)`; normal history mutation no longer requires application code to construct or publish `Plan` directly.
- Added the same undo surface to `SessionDatabase` with explicit write/history permission and session-authority checks.
- Kept `HistoryEntry::undo_plan()` / raw Plan publication as the advanced tooling/protocol path; no second history mutation engine was added.
- Restored and extended hostile public coverage for Γ `F64Total`: `NaN`, `-0.0`, `+0.0`, infinities, exact predicates, and maintained watch delta/no-empty-event behavior.
- Preserved Pass359 relationship mutation DX: `Many`, `OwnedMany`, and filtered selections accumulate directly into `&mut Transaction` while ownership/orphan/derived-preview semantics remain kernel-backed.
- Removed the obsolete short `Transaction::add(Plan)` alias from the ordinary public surface; explicit low-level composition is now only `add_plan`, while normal application mutation stays resource + `&mut Transaction` + payload.

## Pass361 — composed predicates and ordered-boundary DX

- Exposed maintained `TopKWithTies` through ordinary object queries and made conjunction composable without callback/materialization fallback.
- Kept Γ ordering, boundary ties and watch maintenance identical between exact and incremental execution.

## Pass362 — exact count and stronger ordered naming

- Replaced SDK-side `rows/ids -> len()` count paths with kernel `Group + ExactCount`.
- Collapsed ordered-boundary naming to ordinary `top` / `bottom`; their semantics remain tie-preserving rather than exact-k truncation.

## Pass363 — multiplicity-preserving projection and grouped aggregates

- Corrected object `select` to preserve source multiplicity using kernel-native `PromoteToBag -> Project`; explicit `.distinct()` applies maintained Γ quotienting.
- Added typed `group_by(...).count()` and exact-f64 `group_by(...).sum(...)` over the existing kernel `Group`, with live/Candidate parity.

## Pass364 — aggregate query values and maintained aggregate watches

- Changed grouped `count` / exact-f64 `sum` from terminal materialization into typed aggregate query values.
- The same aggregate expression now supports exact `.all()` and maintained `.watch()`; watch events are the exact Group delta, not recomputed snapshots.
- Candidate grouped aggregates use the same result-shape abstraction without introducing a second aggregate engine.
- `one_or_none()` cardinality checks now use kernel `ExactCount` before decoding an object/projection, avoiding full-result materialization merely to decide 0/1/>1.

## Pass365 — aggregate ordered-boundary composition

- Grouped `count()` and exact-f64 `sum(...)` queries can now continue into `top(k)` / `bottom(k)` without materialization.
- Aggregate boundaries lower to the existing kernel `Group -> TopKWithTies` calculus and retain all Γ-equal groups at the boundary rather than truncating to exactly `k` rows.
- Live exact reads, Candidate reads, and maintained watches share the same aggregate-boundary expression; no Rust-side aggregate sort or watch recomputation path was added.

## Pass366 — composite Γ grouping

- Generalized ordinary `group_by` from a single field to one typed group-key law; `(field_a, field_b)` and three-field tuple keys lower directly to kernel `Group { group_columns, group_equivalences }`.
- Composite grouped count/sum keep exact, Candidate, maintained-watch, and `top`/`bottom` semantics on the same query expression; aggregate ordering uses the column after the complete group key.
- No `group_by2/group_by3`, host-language map regrouping, or tuple-specific execution engine was introduced.

### PASS508
- Unified schema-aware prepared publication for mixed relation + field effects.
- Added bounded `SchemaAwareFormationContextWitness`; hosted stale commit no longer uses historical `revision_at(base_revision)` for formation typing.
- Routed unguarded field-only stale publication through the unified prepared walker; retained guard-specific sealing path remains exact and separate.

### PASS510
- Added durable retry-only `SealClientIntent` WAL authority for already-satisfied exact intents without synthetic semantic revisions.
- Recovery/checkpoint retry authority now includes no-op seals in the existing committed-transaction ledger.
- Schema-aware relation/mixed publication seals empty residuals and safely accepts certified same-idempotent model overlaps.
- Schema-aware intent readiness now preserves structured conflict/coordination diagnostics from kernel preparation.

## PASS512
- Added stable formation-proof diagnostic categories distinct from current-world transaction conflict.
- Replaced relational formation-seal global causal-history replay with persistent retained relation-delta lineage.
- Added recovery coverage and release benchmark showing cost tracks relevant relation deltas rather than total retained revisions.

## PASS514 — binding outcome/diagnostic closure

- Added stable `DiagnosticCode::as_str()` spellings for language bindings.
- Python PyO3 probe now exposes structured `CfmdError(code, message)` and exact commit outcomes including `AlreadySatisfied` versus retry `AlreadyCommitted`.
- Built and clean-installed a real CPython 3.13 wheel with maturin 1.15.0; the full asyncio/product hostile probe passes.
- Hosted protocol now preserves `FormationProofUnavailable` and `FormationProofInvalidated` through append-only wire error tags instead of collapsing them to `Internal`.
- Retained the selected migratable-watch DX under `docs/api/CFMD_MIGRATABLE_WATCH_DX_RU.md` without treating the current validation probe syntax as final product API.
- Hostile audit selected granular carrier/lifecycle/keeps-alive publication authorization as the next semantic/security closure.


### PASS515 — exact model-coordinate authorization
- Added exact current-world publication permissions for carrier presence/member, lifecycle entity/root, and keeps-alive presence/edge.
- Generic write authority no longer covers these model coordinates; stale schema-aware publication is checked against the transported exact footprint.
- Added stale A->B runtime/session regression for preview, readiness and commit authorization.

## PASS516
- Exposed deterministic database-wide `ModelRuleExpr` and normalized `FiniteF64` through the public Rust facade.
- Added `SchemaBuilder::model_rule`, lowering into the existing kernel validation/invariant-closure engine.
- Added `#[cfmd(matches = <TextPattern expression>)]` for deterministic persisted text-pattern validation.
- Added public regressions for pattern enforcement and model invariant persistence across reopen.

## PASS517
- Added typed object invariant composition (`Object::rule`, `Field::rule`, `ObjectRuleField`, deterministic `and/or/negate`).
- Added object-first model-rule constructors and `SchemaBuilder::object_rule` lowering to existing `RelationAll` kernel semantics.
- Typed rule coordinates now honor persisted `semantic_name`/`bind` identity; raw semantic IDs are no longer required for normal object invariant DX.

## PASS518
- Added persisted semantic field-to-field equivalence and ordering predicates backed by explicit equivalence/ordering module IDs.
- Removed the late registry-free VMF model-rule evaluation seam; runtime/recovery use the pinned semantic registry.
- Added typed scalar/bool binary-rule DX and root-only direct/optional-reference rule coordinates without exposing deep traversal as row-local validation.
- Added durable reopen and public regressions for ordered, bool-equivalence and reference-equivalence invariants.

## PASS519 — selected exact aggregate invariant measure

- Generalized the exact finite-f64 model invariant from an unconditional column sum to a persisted row-selected measure: `sum(P(row) * value(row))`, where `P` is the existing deterministic `SemanticRuleExpr`.
- Existing `object_exact_f64_sum_range` lowers to the same law with `P=True`; added typed `object_exact_f64_sum_where_range` for selected aggregates without raw relation/column IDs.
- Maintained witnesses update in O(delta rows), using exact inverse removal/addition from `kernel-aggregate::ExactF64Sum`; selector fields join the aggregate column in the exact dependency footprint.
- Semantic selector predicates use the same pinned `SemanticRegistry`; registry-free witness maintenance fails closed to authenticated rebuild rather than comparing through Rust or replaying a query.
- Checkpoint/reopen persistence carries the selector expression with the aggregate invariant.\n\n## PASS520\n- Unified relation cardinality/exists/all into persisted `RelationExactCountRange` backed by `ExactCount`; removed the three legacy variants and their checkpoint tags.\n- Added typed `object_exact_count_where_range`; cardinality/exists/all now lower to the same selected-count law.\n- Preserved exact VMF mass, semantic selector authority, O(delta) witness maintenance, durable reopen, and zero-overhead `count(True)` specialization.\n

## PASS521
- Added persisted exact aggregate-to-aggregate comparison over independently maintained count and finite-f64-sum measures.
- Generalized model-rule dependency indexing to multiple relations and exact touched-side delta maintenance.
- Added exact `ExactF64Sum` witness comparison without host-f64 rounding.
- Added object-first count/sum comparator constructors and durable cross-relation reopen coverage.

## PASS522
- Added persisted Γ-keyed `RelationGroupedExactCountRange` model invariants with sparse exact per-group membership/selected-count witnesses and maintained global violation mass.
- Added semantic-equivalence canonical group keys, delta-local bucket maintenance, checkpoint/reopen support, and typed `object_group_exact_count_where_range`.
- Grouped validation does not use SQL `GROUP BY/HAVING`, query replay, host equality/hash, or dense all-group rescans.

## PASS524 — same-domain Γ-keyed grouped exact measure-product comparison
- Added persisted grouped exact aggregate comparison only for a definitionally shared live group domain: one relation, one group-coordinate tuple, and one Γ-equivalence tuple.
- Each live bucket maintains exact membership plus left/right exact aggregate witnesses and one local comparator contribution; row deltas touch one canonical bucket only.
- Added count/count and exact-f64-sum/sum object-first grouped comparator constructors, durable reopen coverage, and fail-closed schema rejection for cross-relation domains or mixed measure types.
- Group identity remains owned by `SemanticRegistry`; no query join, zero-fill, host hash/equality, or GROUP BY/HAVING fallback was introduced.


## PASS525
- Normalized kernel-persisted grouped invariants to one `RelationGroupedExactMeasure` law with typed exact range/compare constraints.
- Removed duplicated grouped relation metadata and unreleased grouped checkpoint tags; relation authority is derived and schema-validated from exact measures.
- Kept object-first runtime APIs as presentation sugar and retained specialized compiled grouped count/sum/compare hot paths with unchanged sparse Γ-bucket maintenance.

## PASS526 — kernel-wide exact-measure persistence normalization
- Replaced the three kernel-persisted global exact invariant variants with one `RelationExactMeasure { constraint }` law.
- Renamed the grouped-only constraint type to kernel-wide `ExactMeasureConstraint`; global and grouped persistence now share typed Range/Compare vocabulary.
- Preserved global cross-relation homogeneous comparison while applying same-relation/domain alignment only to grouped constraints.
- Checkpoint encoding now has one global exact-rule tag and one grouped exact-rule tag; unreleased legacy global tags were removed.
- Runtime object-first APIs remain presentation sugar; `kernel-validation` still compiles to specialized count/sum/compare hot paths before row maintenance.


## PASS527 — Exact ordered multiplicity / extrema
- Added `ExactOrderedMultiset<K>`: exact multiplicities in an ordered sparse index; insert/delete O(log D), min/max from the index without source rescans.
- Ordered extrema canonicalize values through `SemanticRegistry::canonical_order_key`; host `Ord` is used only on the resulting semantic `CanonicalOrderKey`.
- `ExactAggregateMeasureExpr::OrderedExtremum { Min|Max }` reuses the normalized exact-measure persistence/compare substrate; no parallel top-level model-rule variant.
- Partial extrema comparison is support-aligned: None/None satisfies, Some/Some compares, mismatched definedness violates. Non-emptiness remains an explicit count law.
- Registry-free extrema maintenance is fail-closed; semantic ordering authority is mandatory.
- Deleting the current min/max updates the ordered multiplicity witness locally and survives durable reopen.
- OPEN next: hostile-R&D exact order-statistics / quantiles. Do not accept row rescans, SQL MIN/MAX/ORDER BY fallback, host value ordering, or dense rank arrays.


## PASS528 — exact order statistics
- Rebuilt `ExactOrderedMultiset` as one exact AVL order-statistics tree with subtree cardinalities; rank/select and deletion-safe extrema are O(log D) with no second index.
- Added exact lower-quantile selector `floor(p * (n - 1))` using integer arithmetic, and normalized persisted extrema into generic ordered-statistic selectors.
- Added object-first ordered-statistic comparison and durable reopen regression; failed missing-key deletion now preserves the witness atomically.

## PASS530
- Closed Γ-grouped ordered-statistic range invariants over sparse canonical group buckets and exact AVL order-statistics witnesses.
- Added object-first grouped ordered-statistic range DX and durable reopen regression.
- Marked the current deterministic Semantic Rules line functionally complete; next line is schema-owned authorization policy.

## PASS531
- Added persisted authoritative-schema Role -> AccessCapability -> exact PermissionCoordinate policy.
- Removed ephemeral Role-as-permission-bag product semantics; schema roles compose capabilities/roles by monotone union with cycle rejection.
- Added current-schema role resolution to PermissionSet and typed object capability coordinate helpers.
- Authorization policy survives checkpoint/reopen; runtime enforcement remains the existing exact P503-P515 engine.


## PASS532
- Added authoring-only `AuthorizationPolicy` bundle and `SchemaBuilder::authorization`.
- Added typed many-relationship attach/detach/move capability selectors plus exact global operation helpers.
- Added current-schema `session_for_roles` / `refresh_session_roles` helpers for external role assignments; assignment remains non-persisted.
- Kept PASS531/P503-P515 as the only persisted policy/enforcement authorities; entity/field role metadata is explicitly not a second security model.

## PASS533
- Closed Authorization frontend without adding a duplicate policy macro DSL.
- Added schema-neutral `MigratableQueryWatch` / `MigratableWatchEvent` and `Database::migratable_watch`.
- Added zero-row-rebuild definitionally-equivalent maintained-plan context rebind; ordinary watch stays schema-bound and structural migration fails closed.

## PASS534
- Added retained-program structural MigratableWatch transport for exact row-observation identity migrations.
- Column semantic-ID changes can cross without row/result rebuild when the migration proves same-ordinal identity rows.
- Transported read footprints are reauthorized against target authority.
- Added explicit `MigratableWatchEvent::materialize_schema` late materialization; row permutation/value-changing migrations remain fail-closed.

## PASS535
- MigratableWatch now supports exact relation-coordinate retargeting (`source relation A -> target relation B`) for certified row-identity migrations.
- Retargeting rewrites only semantic scan coordinates and reconstructible differential metadata; maintained rows/canonical lookup are preserved without scan/rebuild.
- Persistent relation base witnesses retarget their relation coordinate while independently checking target row type and canonicalizer identity.
- Added E2E regression over a nested filter query proving a pre-migration row can be removed through the target relation after cutover.

## PASS536
- Added verified observation-transport classification separating row-identity metadata transport from row-local transforms that require maintained-state value conversion.
- Kept value/shape-changing MigratableWatch cutover fail-closed rather than adding query/result rebuild or old-schema routing fallback.
- Added exact regression for `i64 -> f64`: future row-local delta transport remains valid, while already-maintained state is rejected until a non-O(rows) state homomorphism exists.
- Documented the only clean future generalization as factorized observation-state realization; DB 1.0 proceeds to Context/client lifecycle closure instead.

## PASS537 — bounded Context lifecycle and contract representability
- Locked scoped Context behavior across schema cutover: an admitted A Context remains A for the scope, while exact intent publication transports forward into current B.
- Added stable `ContractNotRepresentable` runtime/protocol diagnostics for typed consumer binding incompatibility.
- Isolated post-cutover newly arriving old-client compatibility as the next certified current-world SchemaBridge theorem; no old-schema live routing fallback was introduced.

## PASS538 — certified current-world SchemaBridge foundation
- Added verified `SchemaBridge` read compilation for row-representation-identity migrations, including exact relation-coordinate retargeting with unchanged operator trees and result types.
- Added retained-lineage `CurrentSchemaBridge` resolution to authoritative HEAD without historical state materialization.
- Unified bridge-side exact relation delta/write-footprint transport with the existing migration transport theorem; value-changing old-language reads remain fail-closed.

## PASS541
- Added exact admission-scoped object/reference/non-owned relationship and semantic-precondition bridge lowering into current authoritative schema coordinates.
- Added remote `ContractHead` watch activation and fixed maintained watch state to build from the compiled target expression.
- Kept owned relationship/orphan-policy transport fail-closed pending its own lifecycle certificate.
- Selected Schema-Owned Access Report 2 as the active authorization/DX R&D source.

## PASS542
- Persisted schema-owned exclusive ownership/orphan contracts and added exact migration bridge certification for `OwnedMany`.
- Bridged ownership now requires identity preservation of the relationship relation and target object relation plus exact orphan-policy equality; policy changes fail closed.
- Checkpoint semantic-context codec advanced to unreleased v7 so ownership semantics survive reopen and retained migration lineage.
- Closed the remaining P0 zero-downtime Context/client/watch lifecycle gap; next line is migration frontend/diagnostics.

## PASS543 — exact prepared migration workflow foundation
- Added runtime-owned `MigrationPlan`, `MigrationValidation`, `MigrationPreview` and opaque `PreparedMigration` surfaces.
- Planning/validation avoid target-data construction; preview constructs and validates the exact target without publication.
- Prepared execution is bound to database/source revision and fails stale rather than recompiling against a moved HEAD.
- Existing immediate `migrate` now uses the same prepare/execute implementation.
- Session migration workflow reuses the same `SchemaMigrate` authority and runtime semantics.
- Added structural migration cost classification and initial stable migration diagnostics vocabulary.

## PASS546 — authoritative Schema.Access migration contract
- Replaced the active `AuthorizationPolicy`/`SchemaBuilder::authorization` mental model with authoritative `SchemaAccess` / `SchemaBuilder::access`; the existing PermissionSet evaluator remains unchanged.
- Migration verification now proves exact capability realization and role-composition preservation against the independently authoritative target Schema.Access.
- Access widening/narrowing or role-definition changes fail closed with structured Access-domain migration diagnostics.
- Deferred public `FORMAT_VERSION` until intentional Access policy changes have one explicit durable migration theorem, avoiding immediate released-format churn.

## PASS547
- Added exact Schema.Access migration diff classification, transitive affected-role closure and durable per-capability/per-role policy-change approvals.
- Approved Access changes are encoded in the canonical migration program and reverified on recovery; unused approvals fail closed.
- Added structured runtime projection for approved Access changes.
- Recorded native enum/sum semantics and physical representation as the final pre-format blocker.

## PASS548
- Added exact semantic-sum widening migration: stable variant IDs survive additive enum evolution without rewriting existing values.
- Persisted the widening expression in canonical schema-migration durability metadata and verified reopen.
- Kept sum widening read-only for lens inversion unless a separate certified backward law exists.
- Selected the Memory -> Durable same-revision persistence transition architecture in `docs/rnd/CFMD_MEMORY_TO_DURABLE_TRANSITION_PASS548.md`.

## PASS549
- Added layout-local bit-packed physical tags for sum/enum columns while preserving stable semantic variant IDs.
- Added dense per-variant payload carriers and zero per-row payload metadata for fully fieldless enums.
- Persisted packed sum physical atoms in current unreleased realization codec v4; removed old unreleased v1-v3 decode compatibility.

## PASS550
- Added source-independent `CanonicalPersistenceImage` as the durability bootstrap seam for same-revision persistence realization.
- Added exact single-file staging/reopen verification preserving causal history, retry/idempotency outcomes and current factorized realization when available.
- Memory -> Durable remains fail-closed for retained historical archives, unresolved prepares, replication authority, external freshness and active streaming checkpoint publication until PASS551 closes their image/handoff law.

## PASS551
- Canonical persistence promotion now preserves unresolved prepared-cut and replication authority.
- Active streaming checkpoint work no longer blocks promotion because unpublished checkpoint staging is non-authoritative physical work.
- Retained historical closure and external freshness remain explicit fail-closed boundaries pending portable closure / trust-anchor handoff laws.

## PASS569 — semantic capability lattice / cyclic convergence

- cyclic/query costing now consumes representation-neutral exact semantic cardinality rather than naming the retained statistics family;
- quotient projections expose their exact maintained cardinality, so retaining quotient state no longer requires a duplicate statistics artifact for costing;
- SAMF/ObservableAtom capability metadata now reflects its existing quotient-fiber and exact-cardinality semantics;
- added in-repository R&D and compatibility/hostile/productization ledger updates for the remaining semantic-fiber physical-profile convergence.

## PASS570
- Unified statistics, quotient and SAMF artifact identity as one semantic-fiber artifact with explicit physical profiles.
- Centralized profile capability declarations and unified telemetry/memory-family vocabulary.
- Restored full green `kernel-plan` suite by rewriting inherited arbitrary-target retry regressions against the derived durable request law.
- Added measured retained-memory profile frontier demonstrating non-monotone cost by capability strength.


## PASS571
- Added `kernel-semantics::fiber_carrier::FiniteFiberCarrier` R&D implementation with Measure/ExactFibers/ProjectedFibers profiles, coordinate-class interning, shared joint-fiber signatures, and exhaustive forgetful-law tests.
- Added semantic-fiber build/memory/delta frontier probe; ExactFibers materially reduces quotient retained structure while ProjectedFibers remains memory-gated against SAMF.
- Added `docs/rnd/CFMD_FINITE_FIBER_CARRIER_PASS571.md` and `docs/status/SEMANTIC_FIBER_UNIFICATION_LEDGER.md`.

## PASS572
- Added exact `RowKeyMassRetention` semantic-fiber realizer and moved production quotient retention off unused joint-row buckets.
- Reframed semantic-fiber demand as exact observation capabilities rather than ordinal profile strength.
- Corrected SAMF memory accounting for `RevisionObservableCatalog` and added hostile distribution frontier evidence.
- Adopted retention synthesis / exact-realizer selection as the continuation architecture; SAMF remains until workload/resource selection can replace it without regression.

## PASS573 — competitive semantic-fiber retention compiler

- Added exact observation/resource retention-plan compilation with shared `ResourceFootprint` accounting and deterministic realizer witnesses.
- Added protected-reference performance firewall using conservative read envelopes plus lifecycle/peak guards.
- Protected exact implementations remain selectable even when synthesis has no rule for an observation yet.
- Split global cardinality from keyed joint mass and made slot demands coordinate-indexed.
- Imported the competitive-retention R&D continuation and carried the semantic-fiber ledger forward.

## PASS574
- Decomposed semantic-fiber shared-key retention into `CanonicalJointKeyPool` plus independent `SharedRowKeyRoute` while preserving production quotient semantics.
- Added maintenance-closed physical resource atoms and cycle-safe dependency closure to the exact retention compiler.
- Added a standalone release calibration harness; shared Text256 routing is promising and materially below SAMF, but conservative owned/shared envelopes still overlap, so no protected latency point was removed.

## PASS575
- Restored a protected direct `RowCanonicalKey + JointMass` production engine after fair shared-key calibration showed a real workload crossover; shared canonical retention remains a gated candidate rather than the global quotient lowering.
- Kept the PASS572 removal of unused quotient joint-row buckets.
- Demoted arbitrary-target `RevisionTransitionRequest` from the public `kernel-plan` API to crate-private/test-only preparation machinery.
- Paused further production retention-synthesis expansion pending stable R&D evidence.

## PASS576
- Added structured public FORMAT V1 recovery diagnostics for reopen, backup, strict verification and fresh restore.
- Preserved typed operation/authority/reason plus exact durable-format version and corruption/protocol byte offsets where available; frontends no longer need to parse debug prose.
- Kept all recovery acceptance, FORMAT V1 bytes and restore semantics unchanged; no salvage or second recovery state machine was introduced.
- Kept external-freshness disaster recovery out of the public vocabulary until its certified kernel transfer has a real product authority-transfer entrypoint.


## PASS579
- Renamed the pre-release product key-rotation operation to `Database::reconfigure_protection(...)` and added the restricted `SessionDatabase` equivalent under `DatabaseAdministration::ProtectionReconfigure`.
- Added `RecoveryOperation::ProtectionReconfigure` so durability failures preserve structured PASS576 diagnostics.
- Kept protection administration distinct from schema migration, persistence transition and external-freshness authority transfer.
- Rejected invalid rewrap source/target/algorithm requests before the durability poison boundary; caller misuse no longer poisons healthy committed storage.
- PASS580: restored a fully green Rust product acceptance frontier by separating identity-free `cfmd_object!` value mutation from identity-bearing entity deletion. Exact full-shape value-object deletes now remove selected persisted row values without inventing semantic identity; a field named `id` remains ordinary data. Live `save_as = backup + open` was rejected because persistence images carry operational authority that cannot be duplicated into a concurrently live fork without a separate theorem.

## PASS582 — strict security authority separation
- Split client/data authority (`Schema.Access`, `SessionDatabase`) from database control authority (`DatabaseControlSession`, `AdminDatabase`).
- Removed Schema-owned migration/admin permissions and migration self-approval.
- Added independent schema-publication, access-policy, declassification and migration-data-inspection control claims.
- Added epoch-bound Schema-role session re-resolution.

## PASS583
- Added sealed `MigrationSecurityImpact` with exact source-data/integrity dependencies, per-role observation flows and explicit declassification edges.
- Replaced change-kind-based declassification gating with the exact noninterference factorization theorem.
- Bound migration security identity to canonical durable program bytes + source revision + migration id.
- Made `MigrationDataInspect` conditional on actual sealed source-data dependencies rather than all prepare operations.


## PASS584 — sealed migration approval / control credential lifecycle
- Split mutable database-control credential roots from use-only control sessions: `DatabaseControlCredential` alone rotates/revokes claims, while `DatabaseControlSession` can only consume live claims.
- Added exact `MigrationSecurityApproval`, bound to one live database authority, source revision, migration identity, complete `MigrationSecurityImpact`, impact digest, approver principal and control generation.
- Security-sensitive hosted migration now requires explicit `validate -> approve -> prepare_migration_approved`; approval may come from a different control credential than the publisher.
- Prepare and execute both revalidate approval liveness; credential rotation/revocation invalidates already-prepared publication before side effects.
- Added hostile coverage for independent declassification approval, generation invalidation, exact-impact mismatch and cross-database replay rejection.

## PASS585 — live fork operational identity
- Added an exact live-fork image that carries semantic/history state while re-founding retry, prepared and replication authority through normal store bootstrap.
- Added `Database::fork_to`, `AdminDatabase::fork_to`, `DatabaseControlPermission::Fork` and structured Fork recovery diagnostics.
- External-freshness sources remain fail-closed pending an independent freshness-root bootstrap theorem.

## PASS586
- Closed the externally anchored live-fork exception with independent target freshness-root bootstrap; the source freshness authority is neither copied nor rebound.
- Added sealed first-generation freshness binding and exact response-loss reconciliation for bootstrap publication.
- Added `Database::fork_to_with_external_freshness(...)` and the restricted `AdminDatabase` projection under the existing `Fork` control authority.
- Preserved PASS585 fresh retry/prepared/replication roots and source at-rest protection floor; ordinary open of the anchored target remains fail-closed.

## PASS594
- Integrated proof-checked Set-CQ semantic interning for equivalent multi-root maintained observations without replacing structural/durable query identity.
- Kept Γ-factorized aggregate work in R&D/test/example scope pending production admission proofs.
- Added one replayable replication-authority frame-source abstraction and streamed semantic-base staging directly into existing `CFAS/CFAO` publication instead of retaining the complete encoded frame sequence in memory.
