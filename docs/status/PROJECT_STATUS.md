# Project Status

## Kernel state

The CFMD global kernel hostile/refactor campaign is **COMPLETE / FROZEN after Pass280** for the declared scope. Frozen kernels reopen only on concrete evidence: correctness counterexample, proof/authority seam, measured complexity regression, new mathematical requirement, or a product requirement that cannot be implemented cleanly above the kernel.

The historical problem ledger is closed for the declared scope; the current audit inventory is in [`KERNEL_HOSTILE_LEDGER.md`](KERNEL_HOSTILE_LEDGER.md).

## Active phase — Rust productization

The architecture is Rust-first and now has two deliberate product layers:

```text
Rust applications        Python / .NET / Studio
       |                         |
      cfmd                       |
       \                       /
                cfmd-runtime
                    |
              kernel-* crates
```

`cfmd` is the public Rust application crate. `cfmd-runtime` owns the universal semantic/runtime protocol used beneath it and by future bindings. Internal kernel types remain private implementation details.

### Implemented foundation

- create/open through one `Database::builder` lifecycle;
- object-first schema/query/write DX with explicit reference/cardinality semantics;
- low-level relation/value/query protocol retained as an explicit dynamic escape hatch;
- `Plan -> Candidate -> commit`;
- durable history, exact inverse/redo, historical worlds and certified non-head rebase;
- exact revision-tagged watch with cancellation, lag/catch-up and provider-neutral wake delivery;
- P345-P348 executor-neutral async watch foundation: shared readiness identity + bounded durable drains, race-free `Waker` registration owned by `PublicationNotifier`, direct `watch.next().await` Futures on the runtime watch itself, dependency-frontier wake filtering, observable-event quotienting, and hostile coverage through 2,000 simultaneous pending subscriptions;
- hosted session/protocol/wire/local-transport foundation;
- parity-complete single-file durability with encrypted secure-memory-backed operation on Linux;
- P337 public `cfmd` facade, diagnostic categories and no-kernel-dependency CI gate.
- P338 `CfmdEntity` derive foundation with typed identity/reference inference and consumer compile-fail diagnostics.
- P339 generated many/cardinality declarations with explicit many-valued query operators.
- P342 promotes `Ref<T>` / `Many<T>` to first-class snapshot-bound object relation values, removes backlink ownership from the normal model, lowers object relationships to internal lifecycle-safe edge relations, and supports graph insert plus preserve/replace update semantics.

### Immediate roadmap

1. P348 dependency-frontier readiness and observable quotient: **complete**; unrelated subscriptions remain asleep while O(K) delivery is retained for K relevant consumers.
2. P349/P350 CPython asyncio hostile validation: **foundation proven** through PyO3 over the public `cfmd` facade, including cancellation/commit replay reservation and event-loop-shutdown cancellation.
3. continue Python as a black-box product consumer: broaden concurrent/multi-handle/process lifecycle, transaction/conflict/error, relation/lifecycle and watch stress before freezing the final Python facade.
4. add executor-specific crates only where they expose a measured platform capability ordinary Rust `Future` cannot express; Studio/CLI, backup/recovery UX and release hardening follow incrementally.

The active task inventory is [`PRODUCTIZATION_LEDGER.md`](PRODUCTIZATION_LEDGER.md). The detailed API roadmap is [`../api/RUST_API_ROADMAP.md`](../api/RUST_API_ROADMAP.md) and [`../api/PRODUCT_ROADMAP.md`](../api/PRODUCT_ROADMAP.md).

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

## Pass315 encryption productization status

- Encrypted section read/copy path: bounded 64 KiB authenticated chunks; pre-release whole-section section envelopes are intentionally not retained as a compatibility path.
- WAL/section nonce generation: one random 80-bit namespace per 65,536 messages plus a 16-bit counter; retained microbenchmark evidence is `artifacts/p315/NONCE_BENCHMARK.csv`.
- Product key acquisition: `EncryptionKeyProvider` supports create/open without leaking provider semantics into durability.
- External freshness + encryption low-level parity: COMPLETE for keyed single-file reopen/preflight.
- Remaining key-management payer: wrapped random database master key, provider key IDs, key epochs/rotation and password-KDF adapters.
- Remaining write-path payer: section publication still constructs stored section bytes before generation-table publication; bounded streaming copy/read is closed, fully streaming encrypted publication remains future work if large-section creation requires it.


## Pass316 encryption key-management status

- Provider-backed create now generates a random per-database DMK; provider material is a KEK used only to wrap/unwrap that DMK.
- Provider identity is explicit: 128-bit key ID + non-zero provider epoch; operations are `Create | Open | Rewrap`.
- The current pre-release single-file header owns two authenticated/checksummed wrapped-key slots with monotone publication sequence and database key epoch; internal pass layouts are not treated as released compatibility versions.
- `Database::rewrap_encryption(...)` rotates provider KEK metadata without rewriting generation/WAL ciphertext.
- Direct raw-key mode remains supported as a minimal adapter; synthetic pass-to-pass header compatibility was removed before release.
- Provider snapshots can impose a minimum accepted database-key epoch, giving external key authority a fail-closed complete-header rollback fence after rotation.
- NEXT: make provider-floor advancement transactional/acknowledged with external authority where needed, then close fully streaming encrypted generation publication and directory-encryption parity.

## Pass317 encryption key-authority status

- Pre-release internal header revisions are no longer treated as compatibility versions: the P316 header-only v4/legacy-v3 branch was removed and the single-file header uses the same current `FORMAT_VERSION` as root/generation records.
- Pre-release P314 whole-section encrypted-section compatibility was also removed; the bounded chunked encrypted-section layout is the sole current representation and unknown layouts fail closed.
- `EncryptionProviderKey` now carries an external minimum accepted database-key epoch; wrapped-key open rejects a complete but rolled-back header below that floor before DMK unwrap.
- `Database::rewrap_encryption(...)` returns the newly durable database-key epoch so an external provider can advance its floor only after successful rewrap publication.
- Strict Clippy surfaced and P317 removed several incidental ownership/size issues: large single-file backend/shadow-WAL enum variants are boxed and storage/runtime option helpers no longer copy non-consumed configuration values.
- NEXT: define an acknowledged provider-floor advancement protocol for remote/KMS authorities, then attack fully streaming encrypted generation publication and directory-encryption parity.

## Pass318 acknowledged encryption-authority status

- `EncryptionKeyProvider` now has an explicit durable database-key acknowledgement callback carrying provider key ID, provider epoch and database-key epoch.
- Rewrap authority is ordered as local successor-slot fsync -> external acknowledgement -> predecessor-slot retirement.
- A locally durable but externally unacknowledged successor is a recoverable pending handoff, not unconditional authority: open may still admit the previous provider snapshot, and retry adopts the already-written consecutive successor without another rewrap.
- After acknowledgement, the predecessor `CFKW` slot is zeroed and synced; whole-header rollback remains fenced by the provider's minimum accepted database-key epoch.
- Full regressions cover acknowledgement failure, restart under old authority, idempotent retry, predecessor retirement and complete-header rollback fencing.
- NEXT: close fully streaming encrypted generation publication, then directory-encryption parity through the same AE v1 codec/key hierarchy. Password-to-KEK adapters remain separate product work.

## Pass319 encrypted generation publication status

- Whole-section `StoredSection` staging has been removed from the production single-file writer; encrypted publication allocates at most one 64 KiB AEAD chunk envelope at a time.
- The pre-release generation layout now places the descriptor table after section payloads, allowing ciphertext section digests to be computed during direct-to-file streaming rather than before publication.
- Generation SHA-256 is accumulated in physical write order; publication no longer rereads the entire new generation to calculate its root digest.
- Crash/publication authority is unchanged: partially written generations remain orphan bytes until generation sync and root publication complete.
- Reader validation now binds the footer table offset/length and rejects descriptors crossing into the footer.
- Physical-writer additional memory is now payload-independent, but higher canonical codecs still materialize caller-owned plaintext `Vec<u8>` values (notably checkpoint/metadata encoding).
- NEXT: introduce a one-pass exact-length section-source/canonical streaming encoder so generation publication is end-to-end payload-bounded, then apply the same AE v1/key hierarchy to directory storage. Password-to-KEK adapters remain separate product work.

## Pass320–Pass323 bounded streaming status

- P320: canonical checkpoint/metadata encoding is sink-based and exact-length; SingleFile generation publication no longer requires payload-sized plaintext `Vec<u8>`, and Directory one-shot checkpoint/metadata writes use the same canonical stream.
- P321: Directory resumable checkpointing uses one generation-scoped bounded canonical spool with integrity binding, avoiding restart-from-zero/O(n²) encoding while keeping the spool non-authoritative.
- P322: SingleFile recovery decodes checkpoint/metadata through a bounded `BinarySource`; encrypted sections authenticate one 64 KiB chunk before exposing plaintext, plaintext sections use a 64 KiB read window, and replication-archive replay is frame-streamed.
- P323: SingleFile replication rotation no longer concatenates the old authoritative archive and live prefix into a whole-archive `Vec<u8>`. A composite section source streams the old section plus the exact frozen live-frame prefix directly into the generation writer; streaming-checkpoint cuts retain only the prefix count, not duplicate archive bytes.
- Current remaining payload-sized durability payer: prepared-cut capsule encoding/SingleFile recovery still materializes the complete capsule. Directory AE v1 parity remains blocked until the shared bounded source/sink path covers that final durability payload class.
- Hostile follow-up: P323 removes whole-archive RAM duplication but does not remove historical copy amplification: each rotation still streams the complete retained replication-authority archive into the successor generation. Repeated checkpoints can therefore pay O(retained authority history) per rotation. The next R&D target is a compact canonical replication-authority snapshot/delta law, not another buffering optimization.

## Pass324 replication-authority compaction R&D status

- P324 made the exact replay-produced replication semantic state explicit in executable tests and proved capture/restore equivalence for exercised authority state.
- R&D result: a monolithic semantic snapshot is **not** sufficient to remove the asymptotic copy-amplification class. Replicated effects and revision frontiers are retained causal semantics and establish an `Ω(retained replicated history)` lower bound for an exact snapshot in the worst case.
- Therefore the next production architecture is not "rewrite one smaller archive" and not a generic fallback to the historical journal. It is immutable, content-addressed replication-authority segments: each checkpoint publishes only the new canonical delta plus a stable parent identity.
- Parent identity must be digest/logical-id based rather than raw file offset because single-file physical compaction relocates bytes.
- Required equivalence law for the implementation is `replay(flat historical frames) == replay(flatten(authority segment chain))`, with bounded frame/chunk replay and exact streaming-checkpoint cuts.
- The detailed derivation is in `docs/architecture/REPLICATION_AUTHORITY_COMPACTION.md`.
- NEXT: introduce `ReplicationAuthoritySegmentId`, canonical bounded segment envelopes, and a relocatable segment index; only after executable replay equivalence is established should the P323 monolithic archive-copy path be deleted.

## Pass325 replication-authority segment foundation status

- `ReplicationAuthoritySegmentId` now implements the P324 content/parent-bound SHA-256 identity law.
- `CFAS` is a bounded canonical segment envelope over the existing replication frame grammar; segment replay does not introduce a second authority evaluator.
- `ReplicationAuthoritySegmentIndex` canonically maps stable IDs/parents to relocatable physical extents and rejects cycles, missing/unreachable entries, overlapping extents and conflicting duplicate identities.
- Executable equivalence covers a two-segment authority chain and verifies the same semantic snapshot before and after physical relocation of both segments.
- Parent binding is executable: identical delta bytes attached to a different parent produce a different segment identity.
- Honest remaining payer: P323's active SingleFile checkpoint path still republishes the historical replication archive because the root/generation physical authority does not yet reference immutable external segment extents. The segment layer is executable but not yet the product persistence authority.
- NEXT: P326 should publish immutable segment extents as first-class SingleFile authority using persistent O(new-delta) locator metadata (not a rewritten full index), bind only the current segment/locator root into the generation, recover through the P325 chain, and teach physical compaction to rebuild/relocate the reachable closure before deleting the old monolithic archive-copy path.

## Pass327 authenticated immutable authority-object status

- P326's physical segment prototype remains rejected; no plaintext external replication authority is active in the product path.
- CFMD AE v1 now has an HKDF-separated immutable-object domain in addition to section and WAL domains.
- `CFAO` is a bounded external object envelope for `CFAS`: object kind, plaintext segment identity, parent identity, plaintext length and chunk geometry are authenticated; payload is encrypted one 64 KiB chunk at a time when database encryption is enabled.
- Segment identity is still derived exclusively from canonical plaintext semantics. Nonce choice and physical relocation do not change `ReplicationAuthoritySegmentId`.
- Object AAD excludes physical offset/generation, establishing the compaction law `relocate = exact authenticated byte copy + locator rebuild`, not decrypt/re-encrypt.
- Plaintext/encrypted object modes are explicit and fail closed; there is no downgrade/error fallback.
- The maintained P323 replication archive section is still product recovery authority. P327 intentionally closes the encryption prerequisite before reactivating the P326 locator/root architecture.
- NEXT: reintroduce persistent O(1)-per-delta linked locators/root binding using `CFAO`, recover through the P325 two-pass segment chain, and make compaction copy exactly the reachable authenticated objects before deleting P323 historical archive copying.

## Pass351 product status

Rust product writes now use a database-owned control surface. `Transaction::new()` is an adaptive passive atomic-intent container, but it cannot preview or publish itself: ordinary terminal operations are `db.preview(&tx)` and `db.commit(&tx)` (or the corresponding restricted `SessionDatabase` methods). Candidate is inspectable future state only; raw Plan publication is explicitly named `commit_plan`. Cross-database/session intents fail at the authority boundary. The next DX target is to remove manual Plan plumbing from ordinary entity-set mutations while preserving Plan as the internal/advanced representation.
- P351 also closes the `Session::new`/`PrincipalId` public-facade leak; hosted/session application code no longer needs to import `cfmd-runtime` merely to construct public session authority.

## Pass360 product status

- Ordinary object CRUD, relationship mutation, and history undo share the same `&mut Transaction` accumulation model.
- Database/session objects remain the visible authority for preview/publication; Plan remains an advanced/internal representation.
- Exact durable undo/rebase semantics are unchanged; `db.undo(...)` only removes Plan plumbing from normal application code.
- `F64Total` public exact and maintained paths are hostile-covered for NaN, signed zero, and infinities.

## Pass361 product status

- Object-first queries now expose the existing Γ-native maintained ordered-boundary calculus as `top` / `bottom`; no materialize/sort fallback is used.
- Predicate conjunction is explicit through `ObjectPredicate::and` and lowers to the same exact filter chain already understood by kernel-query.
- Exact watch coverage proves ordered-boundary insert/remove deltas through the public `cfmd` surface.
- NEXT: outside-in projection/aggregate audit, especially typed distinct and count/group capabilities where the kernel already has exact maintained semantics.

## Pass363 product status

- PASS362's assumption that ordinary `select` should expose set projection directly was rejected after DX review: projection now preserves source multiplicity through kernel `PromoteToBag -> Project`; explicit `.distinct()` performs Γ-semantic quotienting through the existing maintained `Distinct` operator.
- Projection `count()` therefore counts projected occurrences by default, while `.distinct().count()` counts semantic classes. Exact watch behavior matches the same law without host-side deduplication or recomputation.
- `group_by(key).count()` now exposes kernel `Group + ExactCount` directly for live and Candidate object queries; `group_by(key).sum(f64_field)` exposes existing `ExactF64Sum` with the same Γ group-key semantics.
- `top` / `bottom` remain the ordinary names for tie-preserving Γ boundary selection; weaker exact-k / arbitrary-row-order semantics remain intentionally absent.
- NEXT: maintained typed grouped-watch DX, then cardinality R&D for `one` / `one_or_none` so validation can stop materializing full result sets without introducing a hidden limit fallback.

## Pass401 product status

- General `MigrationRelationRewrite::Query` no longer has only a dead-end in the factorized architecture. It can be prepared exactly from its `RelExpr::scan_relations()` dependency closure into native target-B relation-column atoms and then installed by an O(schema/layout) cutover.
- The ordinary factorized compose call remains fail-closed when preparation is absent. There is no current-world schema-A query fallback.
- Current-schema writes can install bounded B-native field/relation physical overlays after semantic write certification; no inverse migration is required, including for non-injective derived values.
- Hostile boundary retained: arbitrary global query outputs do not automatically have stable writable row identity. Universal B-native relation delta/endpoint coordinates remain open and must reuse kernel-change/query semantics.
- 100k bag-Union warm release runs: preparation 27.471–28.930 ms; cutover 39.769–43.053 us.
- NEXT: factorized/streaming preparation executor over the same RelExpr algebra + universal global-relation B overlay; only after those semantics stabilize should runtime/durable realization integration proceed.

## Pass402 product status

- The general-relation write problem is semantically closed more strongly than P401 assumed: after migration cutover a target relation is an ordinary current-B semantic coordinate, not a permanently writable view back into schema A.
- `PreparedRelationRewrite` now exposes its bound relation identity, and `kernel-realization` can consume an exact prepared B rewrite and detach that relation into native B columns. Stale-base support is rejected exactly by the existing Γ relation rewrite authority.
- The law is query-kind independent: `Union`/`Distinct`/`Group`/Bag provenance does not create separate write routes and never requires inverse migration.
- Hostile performance rejects whole-relation detachment as the production hot path: a one-row write over a 100k-row bag relation costs 92.918–98.899 ms in warm release runs.
- NEXT: represent the same prepared B relation rewrite as a bounded immutable Γ-class/multiplicity physical overlay, then lower scans/points and compaction without reintroducing source-schema semantics.

## PASS403 update

Prepared general-relation targets now support bounded current-B physical delta overlays. Exact `PreparedRelationRewrite` semantics remain kernel-query-owned; realization updates persistent Γ-aware occurrence routing plus immutable delta atoms and never falls back to full endpoint detachment or inverse migration. Compaction returns overlays to native factorized columns. The remaining immediate R&D debt is unifying semantic support and physical routing indexes to reduce one-time write-overlay preparation cost before runtime integration.

## PASS410 update

Set `Difference` now reuses the complete Γ-canonical row keys already computed by exact support subtraction to emit `RelationOccurrenceCertificate` together with its output. `RelationBaseWitness` adopts that persistent occurrence root directly, eliminating the post-evaluation recanonicalization pass for this operator. `AntiJoin` and Set-producing `Join` remain intentionally uncertified in the direct executor: their current algorithms own only blocker/join-coordinate canonical keys, not complete output-row occurrence evidence. The next R&D target is compositional child-certificate propagation/handle algebra rather than moving witness construction inside those operators.

## PASS411 update

Set `AntiJoin` now composes full-row Γ evidence from a certificate-capable left subtree and computes only blocker-coordinate semantics itself. Hostile benchmarking rejected nested persistent occurrence roots (1.51–2.28x baseline), so execution certificates now compose as transient row-aligned canonical full-row keys and materialize exactly one persistent `class -> StableRowHandle` root at the final physical adoption boundary. Warm AntiJoin runs are ~0.882–0.979x the old evaluate+final-witness path, while P408–P410 Union/Distinct/Project/Difference certificate behavior remains in its accepted performance class. Direct `Scan` still cannot synthesize evidence in the standalone exact executor; the next line is Set Join composition followed by seeding certificate evidence from the universal current physical Scan authority.

## PASS412 update

Set `JoinEq` now composes exact full-row Γ evidence from certificate-capable Set children: the joined canonical row key is the left child key concatenated with the right child key along exact join fibers, so Join no longer canonicalizes complete output rows a second time. Internal child handles are not treated as output identity; one dense target occurrence root is still materialized only at the final adoption boundary. Hostile performance initially exposed a ~1.09x cheap-I64 regression, which was not accepted; replacing certificate-Distinct's node-heavy `BTreeMap` quotient with contiguous `Vec + sort + dedup` and skipping redundant final sorting for already strictly-sorted keys produced late warm Set Join ratios of ~0.807–0.878x baseline. Direct `Scan` remains intentionally uncertified in the standalone executor; the next target is seeding transient evidence from the already-owned current physical relation witness so ordinary `Join(Scan, Scan)` can use the same law without recanonicalization.

## PASS413 update

Physical Scan occurrence seeding is now exact in `kernel-query`: an existing `RelationBaseWitness` plus exact logical stable-handle order can seed transient full-row canonical evidence, enabling compositional `Join(Scan,Scan)` and `AntiJoin(Scan,...)` without rerunning Γ canonicalizers. Hostile performance rejected automatic product/runtime adoption: even after replacing O(N log N) reverse mapping with dense slot scatter/gather, 100k cheap-I64 seeded Join remained ~1.32–1.38x baseline because rebuilding row-aligned evidence costs ~32–34 ms per execution. The temporary realization auto-wiring was reverted. NEXT: make row-aligned Scan evidence a structurally shared/current-relation-owned representation that advances with the same witness/delta authority; only then repeat perf and wire runtime.

## PASS414 current relation evidence status

Current factorized relations now retain structurally shared row-aligned Γ evidence with their exact base witness/overlay. Exact Scan certificate execution receives a persistent O(1)-clone rather than rebuilding row evidence O(data) per query. Bounded relation writes update the evidence with the same physical swap-remove/append transition used by the overlay. On the 100k cheap-I64 Scan self-Join hostile, seed handoff fell from ~32–34 ms reconstruction to ~0.10 ms shared clone; late warm end-to-end returned to ~1.095–1.097x baseline. Runtime Query/Watch/Change auto-wiring remains next, after a mixed nested-tree hostile.

## PASS415 nested certificate-tree status

Nested Set exact execution now propagates child canonical evidence through Union, Difference, Set Project and redundant Set Distinct instead of recanonicalizing intermediate rows. Together with P414 shared physical Scan evidence and P411/P412 AntiJoin/Join composition, an audited 100k `Scan -> Join -> Project -> Union -> AntiJoin -> Difference -> Project -> Distinct` tree runs at ~0.832–0.896x the old evaluate+final-witness baseline across two release series while still materializing only one persistent occurrence root at the final target boundary. General relation physical preparation now consumes already-owned source Scan seeds automatically when the prepared expression proves seeded certificate capability. Full live Query/Watch/Change authority wiring remains OPEN for PASS416; no runtime-side Γ cache or migration-epoch router was added.

## PASS416 runtime authority update

Current live exact Query now consumes bundle-owned persistent Scan Γ evidence when its prepared algebra proves seeded-certificate capability. `RuntimeRevisionBundle` publishes that evidence atomically with logical revision, physical store, relation base witnesses and maintained materializations; relation transitions update it from prior canonical evidence plus delta rows rather than recanonicalizing the unchanged base. 100k one-row publication costs ~4.8–5.0 ms warm, so the remaining immediate debt is replacing the linear survivor projection with a bounded/order-statistic persistent sequence. Direct physical-handle binding into product `QueryWatch` was hostile-rejected because durable watch replay owns semantic deltas rather than storage-resolved handle receipts; semantic persistent Watch seed handoff is the next runtime target.

## PASS417 witness-owned logical Scan evidence

P417 removes the P416 O(N) Set Scan-evidence publication projection. `RelationBaseWitness` now carries an ordered persistent occurrence view keyed by its monotone semantic `StableRowHandle`; for Set relations this order is exactly initial row order with deletions preserving survivors and insertions appended. Runtime Change publication advances the witness once and obtains the next `RelationScanOccurrenceSeed` as an O(1) persistent view. In the 100k one-remove/one-insert hostile benchmark, the previous dense survivor projection measured ~5.9–7.6 ms while witness-owned publication measured ~0.005–0.009 ms after warm-up (first run ~0.92 ms).

`QueryWatch` now has a semantic seed handoff: maintained Scan canonical lookup can bootstrap from runtime Scan evidence without attaching physical handles or a physical base witness. Durable historical catch-up remains semantic delta replay. Full kernel-query, kernel-plan and cfmd-runtime watch/end-to-end regressions pass. Bag duplicate occurrence order and deeper maintained-index structural sharing remain open and must not inherit the Set proof without a separate law.


## PASS418 Bag occurrence-order authority

P418 closes the Bag duplicate-order gap left by P417. `RelationBaseWitness` now stores each Γ-class in a persistent FIFO occurrence bucket with an amortized-O(1) head offset, so witness removal selects the same oldest live semantic occurrence as logical Bag delta application. This makes witness-owned row evidence valid for both Set and Bag without a second multiplicity/order cache. Runtime bundle Scan seeds are now published for Bag relations from the same authority. On a 100k Bag with 1024 repeated classes, legacy dense survivor projection costs ~4.97–6.27 ms per one-row publication; witness-owned FIFO publication is ~0.005–0.007 ms warm after a ~0.886 ms first run. A 128-transition Bag->maintained-Group hostile keeps witness evidence, maintained Group output and exact recompute coherent through FIFO compaction. Maintained Group itself still does not export a full output-row occurrence certificate: it owns group-key lookup and aggregate state separately, and P409's fresh-output-root regression remains rejected.


## PASS419 retained runtime-root authority status

- `RuntimeRevisionBundle` no longer mirrors witness-owned semantic Scan evidence in a separate `relation_scan_seeds` directory. `RelationBaseWitness` is the sole current/retained Γ occurrence + logical-order evidence owner; Query/Watch derive an O(1) persistent view from the witness in their immutable snapshot.
- Retained-reader hostile proves an old snapshot preserves old Bag witness/evidence after a later atomic publication while the new snapshot observes the advanced root. Derived seeds structural-share the exact ordered-evidence root of their own witness.
- Runtime maintained materialization bootstrap and late registration now consume witness-owned semantic Scan seeds before storage-row attachment, removing redundant Scan canonicalization without binding historical maintenance to current physical handles.
- Independent logical-delta seed advancement is test-only reference code; production publication advances the witness and derives the view.
- Immediate next physical target: quantify retained persistent-node memory/reclamation under many pinned roots, then resume factorized/streaming general preparation and workload-weighted overlay compaction before durable PhysicalAtoms/RealizationRoot.

## PASS420 retained-root / compaction status

P420 closes relation-witness persistent-node retention/reclamation under long-lived roots and removes redundant Γ recanonicalization from factorized overlay compaction. Across 65 pinned 4,096-row witnesses, retained structural storage is ~1.82% of naïve full-copy storage for Set and ~2.22% for Bag FIFO; a 33-snapshot runtime lineage retains ~3.31% of naïve witness-node storage, and weak probes prove oldest-root unique nodes reclaim after drop. Factorized compaction now rebinds the existing certified physical Scan key order onto fresh dense handles instead of rebuilding the witness from materialized rows. An initial semantic-order rebind was hostile-rejected because physical `swap_remove` order can diverge from witness logical survivor order. Repeated 100k/depth-1024 release runs place certified witness rebind at ~0.724–0.845x legacy witness rebuild cost. Whole runtime/history/watch-root memory and workload-weighted compaction policy remain open before durable PhysicalAtoms/RealizationRoot.

## PASS421 status — whole-root retention census and transition evidence reuse

P421 extends P420's witness-only retention proof to the complete immutable runtime root and finds the next real owner: changed logical relations in `Revision/model` still allocate full row buffers. A 33-snapshot/4,096-row Set hostile retains 33 distinct logical buffers (135,168 row slots) while the witness structural union is 17,920 nodes. The storage-resolved transition path now canonicalizes changed rows once and shares those exact Γ keys between maintained Scan indexing and `RelationBaseWitness`. A generic workload-weighted compaction decision law is also encoded; fixed depth/N policy remains rejected. PASS422 should make logical relation state persistent/path-copy bounded before durable physical roots are introduced.

## PASS422 status
Persistent logical relation authority now supports exact relation-only successors as immutable base + persistent removals + persistent inserted tail. Hostile authority census: 33×4,096 naïve row slots = 135,168 versus 528 delta-row upper bound + 225 persistent delta nodes. Whole-root memory is not yet closed because the old borrowed-Vec compatibility surface can retain strong full materializations in historical roots (32/33 observed). Next: remove that façade/cache and re-run whole-runtime reclamation before durable PhysicalAtoms.

## PASS423 status — whole logical-root retention closed

P423 closes the blocker found by the frozen P422 checkpoint. Persistent logical relation roots no longer own a lazy strong contiguous-row compatibility cache and no longer dereference as `Vec<Row>` on immutable reads. Read consumers use persistent iterators/point access; algorithms that genuinely require complete rows request an explicit transient owned materialization at their own boundary. In the 33×4,096 pinned-runtime hostile, all 32 persistent successor roots remain non-materialized even after explicit full reads; authority remains 528 delta-row upper bound + 225 persistent logical nodes versus 135,168 naïve row slots. A weak logical-root probe additionally proves oldest-delta node reclamation after dropping its snapshot. The next immediate productization line is factorized/streaming global `RelExpr` preparation, followed by maintained position-index ownership; durable PhysicalAtoms/RealizationRoot remains gated until those are clean.

## PASS424 status — maintained streaming rejected for one-shot global preparation

The obvious universal reuse path was tested rather than assumed: replay factorized source rows through the existing exact maintained Γ-DTC engine and consume its exact output deltas into physical preparation. It is semantically valid, including output retractions, but it is the wrong ownership model for one-shot work. On the 100k Bag Union hostile it measured **3199.062 ms** versus the accepted current **311.676 ms**; removing the physical sink still left **1907.967 ms**, proving the major payer is maintained Scan/persistent position authority.

The prototype was reverted. The selected next architecture is a separate storage-neutral one-shot relational execution source/sink boundary. It must consume factorized ranges and existing Γ evidence directly, retain only operator-inherent state, emit physical target columns/segments plus exact occurrence evidence, and fail closed for missing lowerings rather than fallback to full-row evaluation. Durable PhysicalAtoms remain gated behind this boundary.

## PASS425 status — one-shot global relational execution accepted

P425 implements the architecture selected by P424 instead of reusing maintained Query/Watch state. General relation preparation now has a storage-neutral `RelExecutionSource -> one-shot operator lowering -> RelExecutionSink` boundary. Scan/Union/Difference/AntiJoin are exact; unsupported operators fail closed rather than invoke the legacy complete-row evaluator. Difference retains only Γ blocker counts and AntiJoin only blocker-key support. Direct Bag Union(Scan, Scan) has a factorized column-range lowering and the target witness is built directly from the output columns.

100k Bag Union release runs measured **284.239 / 265.027 / 245.619 ms**, compared with the prior accepted **311.676 ms** and the rejected P424 maintained-streaming **3199.062 ms**. The next gate is operator/evidence closure (Filter/Project/Set/Distinct/Join, then Group/TopK where exact) plus maintained position-index ownership; durable physical roots remain deferred until that is stable.

### PASS426 — one-shot relational operator coverage

The storage-neutral one-shot executor now covers deterministic equality/order filters, equality-column filters, Bag/Set Project, Set/Bag Union, Distinct, Difference, AntiJoin, JoinEq and PromoteToBag. Exact Join uses one canonical-key fiber index and streams the opposite side; nested Set execution composes without invoking the old complete-row evaluator. 100k exact-I64 Join warm measurements were in/below the generic evaluator cost class.

The immediate blocker before claiming evidence closure is duplicate final Γ work: Set quotient/subtraction operators already own exact full-row canonical keys, while `RelationBaseWitness::build_columnar` canonicalizes their final rows again. The selected next architecture is an opaque kernel-query-owned row-aligned evidence carrier; raw forgeable key injection is rejected. Group/TopK remain fail-closed meanwhile.

## PASS427 status — sealed Γ evidence reaches physical witness

One-shot quotient/subtraction preparation no longer computes a canonical row key for semantics and then recomputes it in the target witness. Exact row evidence is issued by an opaque `kernel-query` authority and adopted as a persistent occurrence certificate; forged/mixed authority fails closed. Maintained canonical position indexing also shares a single canonical-key payload per Γ class instead of duplicating the payload in every row-position entry. Group/TopK exact one-shot lowering and nested pass-through evidence propagation remain the next gate before durable physical roots.

## PASS428 status — one-shot relational execution gate closed

Every current `RelExpr` operator now has an exact storage-neutral one-shot lowering. P428 adds Group with exact aggregate state and TopKWithTies bounded to K plus boundary ties, and propagates sealed Γ row evidence through unchanged-row Filters and AntiJoin. The executor is exhaustive and the former runtime unsupported-operator escape hatch is removed. P427 shared canonical-position keys also beat the frozen P426 payload-copy implementation substantially in 100k release build/churn hostiles. The realization/query-preparation line is therefore stable enough for the next pass to begin durable PhysicalAtoms / durable RealizationRoot and same-semantic-revision crash-safe root publication.

## PASS429 status — first durable physical realization root

P429 crosses the first durability gate for Physical Realization Algebra. `kernel-durability` now owns a versioned durable physical image containing semantic `RevisionId`, only reachable `PhysicalAtoms`, and a direct factorized `RealizationRoot`; reopen reconstructs and validates that physical authority directly without first reconstructing full logical `DatabaseState`. The image uses the existing single-file `PhysicalArtifact` section, so generation authentication and optional AES-256-GCM-SIV protection apply. A same-revision hostile proves unpublished tail data is ignored, while a committed generation/root switch changes physical dependencies without changing semantic revision. Durable relation column order preserves declared ordinal order rather than sorting stable semantic IDs. Next: bind this image into `DurableRevisionStore` checkpoint/recovery ownership and carry it through real rotation/compaction crash matrices before historical root/atom reachability.

## PASS430 status — physical realization enters DurableRevisionStore checkpoint authority

P430 moves the P429 CFPR image under real `DurableRevisionStore` ownership for the default single-file backend. The store now reopens an optional checkpoint-bound `DurableFactorizedRealization`, verifies its embedded `RevisionId` exactly matches the logical checkpoint cut, carries it through ordinary checkpoint rotation and active-generation compaction, and snapshots it into streaming checkpoint publication. Streaming hostile proves the key authority split: a physical root for checkpoint `R0` remains bound to `R0` while exact carried WAL advances recovered durable head to `R2`. Explicit directory physical-root publication is intentionally fail-closed until an equivalent generation/checksum carrier exists. Next: close that backend-neutral carrier/streaming-I/O seam, then move retained history from generation pins to `HistoricalRevisionRoot -> RealizationRoot -> PhysicalAtoms` reachability.

## PASS431 status — durable realization carrier is backend-neutral and streaming

The checkpoint-bound Physical Realization authority is no longer single-file-specific. Directory generations publish a streamed, fsync'd CFPR prerequisite bound by manifest-checksummed metadata to exact revision/length/CRC; reopen fails closed if the bound artifact is missing or corrupt. Streaming checkpoints preserve physical cut authority independently from newer carried WAL. Both single-file and directory CFPR publication/reopen now use bounded streaming/counting codec paths rather than a whole-image `Vec`. The next durability target is historical realization-root/PhysicalAtom reachability and GC sharing, not another storage backend shim.

## PASS432 UPDATE — historical roots enter the shared durable atom graph

P432 introduces durable retained historical realization roots without creating a second history/physical store. CFPR v2 owns one immutable `PhysicalAtomStore`, one current `FactorizedRealizationRoot`, and zero or more causal historical roots keyed by `RevisionEffectId`. Encoding retains only the union of atoms reachable from current + retained historical roots. Synchronous and streaming factorized checkpoint publication inherit roots named by live historical anchors; explicit historical release removes the root and immediately prunes old-only atoms. Reopen/codec hostile proves shared atom IDs survive roundtrip and old-only atoms disappear after release.

The conservative P381–P386 generation archive is intentionally **not yet removed**: `HistoricalEpochMaterial` still needs its historical semantic registry/checkpoint/WAL authority. PASS433 must bind those semantic/causal pieces to the retained realization root before root-backed anchors are excluded from generation pinning. This preserves correctness while moving physical retention from whole-generation ownership toward exact root/atom reachability.

## PASS433 UPDATE — complete HistoricalRevisionRoot authority

P433 closes the semantic/recovery piece that still forced P381–P386 whole-generation retention. CFPR v3 historical roots can carry exact historical `SemanticContext` in addition to causal source `RevisionId` and root topology over the shared atom graph. The durable `SemanticRegistry` remains the single module authority. Runtime history now prefers reconstructing the exact source `Revision` directly from the retained realization root; old checkpoint/WAL `HistoricalEpochMaterial` remains only for legacy/context-incomplete roots.

Generation retention is now conditional on root completeness. Directory compaction physically removes the old checkpoint/manifest while historical reconstruction still succeeds after reopen. Single-file hostile additionally removed unconditional outgoing historical archive creation: the archive is created only when the generation remains in the post-capture pin set. Thus complete root-backed history no longer retains a parallel whole-generation physical authority.

## PASS434 UPDATE — historical publication crash law + lazy read substrate

Complete root-backed history now survives an explicit aborted-streaming/reopen/retry hostile: the root cannot become authority before checkpoint publication, legacy generation authority stays pinned across the interrupted attempt, and only a successfully published CFPR releases the generation. A 64-history-root fixture confirms shared-graph retention rather than per-root atom copies.

`kernel-realization` also exposes direct factorized execution of the full current relational IR without constructing `DatabaseState`. Runtime historical reads are not switched yet: `PhysicalAtomStore` is currently a deep-cloned `BTreeMap`, so embedding a cloned store into every historical `ReadContext` would regress memory. Next work is immutable/shareable atom ownership followed by a unified representation-neutral read authority.

## PASS435 — immutable shared atom/read authority

Physical atom snapshots and retained historical read handles now share persistent map topology and Arc atom payloads. Complete historical realization roots can back ordinary `ReadContext` queries directly through the factorized one-shot executor, avoiding full historical `DatabaseState` materialization. Sparse physical rewrites retain O(path-copy) map structure; exact branch-only allocations reclaim when no root/read handle owns them.

## PASS436 — factorized intermediate history
Complete retained historical realization roots can now advance through exact reversible relation-only history without eager whole-world reconstruction. The runtime derives a new immutable factorized read snapshot by applying only the touched relation delta and structurally sharing all unaffected atom authority. Mixed model/schema transitions still use exact logical history reconstruction until their factorized lowering is proved; no approximation or fallback engine was introduced. Direct object/ref/point reads already use the shared PreparedQuery/ReadContext path, so they inherit factorized authority automatically.

## PASS437 UPDATE — mixed ordinary history is fully factorized
P437 closes the remaining ordinary reversible-history gap identified by P436. `DurableModelDelta` now acts directly on durable factorized read authority: lifecycle, carrier membership/presence and field columns are rewritten as new direct physical endpoints, while untouched atoms remain persistent/shared. `factorized_read_snapshot_at` composes this law with exact relation deltas for `MixedRevision` effects and still refuses schema/semantic-change/full boundaries. Model-only and mixed relation+model hostile fixtures match the exact logical target without constructing a full historical `DatabaseState`.

The storage/history expansion line is now considered closed absent new hostile evidence. Immediate productization returns to Context reference/relationship patches, field-granular change coordinates, granular DB-owned authorization and Semantic Rules/invariants. P435 transient/wide-tree bulk R&D remains deferred performance work.

## PASS438 closure
Context partial patches now support scalar, required-reference, optional-reference and relationship mutations without hidden-field rewrite. Field intents persist exact owner+field coordinates through WAL/history, independent fields rebase by semantic reapply, same-field writes conflict, and rebased commits retain original client intent for idempotent retry. Migration/storage/history line remains closed absent new hostile evidence; next product line is DB-owned granular authorization.

## PASS439 UPDATE — semantic granular authorization substrate
P439 returns productization to the deferred authorization line after P438 field coordinates. `kernel-query` now derives an exact semantic read footprint from `RelExpr`, and runtime session authority enforces the same relation/field coordinates for ordinary, prepared, candidate, historical-factorized and watch reads. Session grants now support relation/field granularity; object relation columns use stable semantic field IDs rather than ordinal identities, and P438 object field patches are checked against the same coordinate at commit. A hostile proves projected-field read and field-only patch grants do not imply full-row/full-relation authority. Create/delete/relationship action classification and write-only patch formation remain the immediate P440 closure target.

## PASS448 UPDATE — quantified/exact aggregate model-wide invariants
P448 extends the P447 model-wide invariant substrate with persisted `Exists`/`All` row quantifiers and an exact finite-f64 sum range, all sharing stable semantic relation-column identity. `CompiledRulePlan` now carries exact relation/column dependency certificates and a relation-indexed rule table; fail-closed validation early-exits while VMF retains exact zero-law accounting. Exact floating bounds compare against the superaccumulator without rounded-sum authority. Checkpoint/reopen carries the new rules. Next: consume column dependencies at the P438 field-coordinate mutation boundary and build maintained incremental witnesses for hot quantified/aggregate rules.

## PASS460 continuation update — historical Γ-support transport + witness-native Set residualization

P460 removes both O(relation) payers left by P459's stale remote exact-effect path. Formation validity no longer reconstructs a historical logical `Revision` or rebuilds `RelationBaseWitness` from historical rows. A restricted `RelationSupportWitness` is projected O(1) from the current persistent Γ witness and rewound through exact durable relation deltas; it exposes support/multiplicity validity only, never scan-order or physical-position authority. Schema/opaque/non-plan boundaries remain fail-closed because support transport across a semantic-context change requires an explicit bridge law.

`certify_transition_rebase` also no longer reconstructs historical revisions merely to derive Γ write footprints. It first rejects semantic/opaque boundaries; when the path is exact and semantic-context-preserving, all proposed/intervening footprints are derived in the shared current semantic context.

Current stale-Set residualization no longer materializes/scans the whole current relation. `RelationBaseWitness` resolves only touched Γ classes to persistent logical positions; the runtime point-reads exact current representatives for removals. Bag residuals remain exact multiplicity deltas unchanged. Hostile coverage uses case-insensitive Set equivalence to prove a stale removal spelled with a different representative removes the exact current row while commuting across a disjoint intervening insert.

Immediate continuation: hostile/perf measurement for long historical effect chains and repeated stale intents, then remote-reader schema-evolution DX if no new transaction payer appears. Typed field-grant sugar remains separate cleanup; all deferred Context/Rules/migration/frontend/bindings/backup/perf/Windows items remain carried.

## PASS461 UPDATE — history depth is now a measured product payer
P461 benchmarked the P460 stale remote exact-effect path through 32/128/512/2048 retained exact transitions. The no-index design is semantically clean but not depth-independent: pre-change total stale commit reached ~22.86 ms at depth 2048. Formation validation was also doing unnecessary full causal-ideal reconstruction plus adjacency/BFS. It now follows the authoritative durable target->source state-transition lineage directly, removing that avoidable traversal while the final depth-2048 median still measures ~21.19 ms (18.23–25.87 ms across three warm runs) without any cache or second history authority.

The remaining linear depth cost is still material: historical Γ-support must currently rewind every intervening touched delta, and `certify_transition_rebase` still scans/reconstructs the causal suffix. P462 therefore owns a measured R&D requirement rather than speculative caching: persistent structurally shared revision-bound support roots plus exact aggregated write-footprint authority, both derived from and GC-bound to the existing durable causal ledger. No serialized witness cache, full-state fallback, or protocol merge engine is acceptable.

## PASS464 UPDATE — schema-boundary effect/guard proof primitives
P464 productizes the security-critical R&D finding that write-effect rebase and passive guard stability are independent proofs. Runtime guard dependencies now use the P462 coordinate timeline and can be bound to residual publication on the same immutable source root; the existing publication seal closes the check/publish race. `kernel-transport` now transports row-local exact relation deltas directly through migration programs, transports field dependencies from certified provenance, and exposes untouched merge inputs as implicit passive dependencies. General/global migration rewrites remain fail-closed. P465 owns the remaining orchestration across one or more durable `SchemaMigrationExact` records and retained epoch indexes.

## PASS466 UPDATE — client intent is no longer realized-world identity
P466 rejects the tempting dual-field patch and establishes a canonical durable client-intent projection. Relation/mixed direct and residual commits compare through `DurableClientIntentView`: original semantic revision + original exact effect + guard identity, excluding revision endpoints, realized residuals/complements and implementation deployment artifacts. Residual WAL v13 carries client and realized semantic revisions separately, while publication/recovery still validates the realized side against `DurableRevisionChange`. Schema-aware retry now resolves committed A intent before any history or migration work; hostile coverage proves A->B commit, B->C migration, reopen, then exact A retry returns `AlreadyCommitted`. Remaining representation debt is explicit: residual intent variants still duplicate realized payload bytes and P467 should eliminate that duplication rather than preserving a mixed authority container.

## PASS470 UPDATE — one current directory checkpoint representation
Directory bootstrap, synchronous rotation and resumable checkpointing now converge on the same current chunk-root + chunk representation. The old flat monolithic directory checkpoint reader/writer and unreleased legacy manifest/checkpoint decoders are removed. Manifest/checkpoint/metadata markers are explicitly format tags, not pass-to-pass compatibility versions. Current-format hostile coverage proves synchronous create/rotate reopen through chunk authority and external-freshness fork detection operates on the current chunked representation. Next pass is an active pre-release legacy/compatibility hostile sweep outside durability; after that, retained per-schema-epoch P462 support/action authority remains the next measured transaction-performance payer.


## PASS472 UPDATE — one semantic-fiber physical authority
P472 removes the superseded `MaterializedSemanticIndexState`/`LegacyIndex` family end-to-end. Semantic Filter/Join fiber execution now has one persisted physical authority: SAMF/`ObservableAtom`. The old advisor, durable recipe, delta-maintenance path and runtime fallback are gone; `SemanticIndexBinding` survives only as a semantic coordinate. Recovery tests now respect the actual artifact laws: ObservableAtom canonical-key cores rehydrate directly, while rebuild/deferred budgets are exercised by rebuild-only I64/statistics artifacts. Immediate work returns to the measured cross-schema stale-transaction payer: retained per-epoch P462 support/action roots.

### PASS473 status
Schema-aware field intent transport now retains one immutable P462 support/action root per live schema epoch. Migration boundaries preserve structurally shared source field roots and semantic context, so stale field intents cross old epochs without durable-history replay or `revision_at()` on the commit hot path. Reopen rebuilds the retained roots from canonical causal/historical authority. Next: row-local relation exact-effect transport through the same epoch-root law.
