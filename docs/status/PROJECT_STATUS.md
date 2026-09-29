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

Rust product writes now have a first-class exact-snapshot `Transaction` composer (`Database::transaction` / `SessionDatabase::transaction`) while retaining one Plan/Candidate/commit authority. Multi-plan writes can be previewed and published without manual aggregate-plan/transaction-id plumbing; cross-snapshot composition and stale publication remain fail-closed, and unchanged publication can be retried idempotently. Python remains an external regression consumer and passes after the Rust change. Next product target: Γ-native ordered/range predicate algebra, not a generic SQL/post-filter fallback.
- P351 also closes the `Session::new`/`PrincipalId` public-facade leak; hosted/session application code no longer needs to import `cfmd-runtime` merely to construct public session authority.
