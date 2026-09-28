# Security Notes

CFMD contains authenticated deployment, replication evidence and durable-store trust boundaries. Security-sensitive changes should preserve fail-closed behavior and use established cryptographic implementations rather than project-local cryptography.

The vendored dependency closure is intentional for reproducible/offline builds. Dependency changes should update `Cargo.lock`, vendor sources and the relevant audit/evidence record together.

Durability certification is not a universal hardware claim. Only exact certified platform profiles are admitted as certified operation.

Cargo-vendor checksum metadata is part of the repository integrity boundary. `bash ./scripts/verify-repository.sh` validates vendored file presence/checksums before Rust CI.

The planned local tooling/Studio endpoint is not implicitly trusted because it is local. It must be opt-in, per-user where possible, capability-token protected, explicit about read/write authority, and route writes through the same Plan/Candidate/revision pipeline as in-process clients.


## Hosted session boundary

`Database` is an unrestricted in-process capability intended for trusted embedded/host code. Untrusted hosted clients must be represented by `SessionDatabase`; raw `Database` must not be exposed across a protocol boundary. Authentication establishes `PrincipalId` and grants outside the runtime. Runtime authorization propagates one shared session authority through derived read/write/history/watch product values; grants may be refreshed by the trusted host, while revocation is monotone and invalidates stale product values. Localhost, local IPC and transport identity are never implicit trust signals.


## Hosted protocol boundary

`cfmd-protocol` accepts only `SessionDatabase`, never unrestricted `Database`. A transport must authenticate a peer and obtain host-issued session grants before constructing `HostedSession`. Localhost, local IPC, process identity and possession of a database path remain non-authoritative trust signals.

Protocol ingress performs its own complexity admission before runtime execution so a future parser/transport cannot accidentally bypass query/mutation resource bounds. Public protocol errors use stable codes; internal/recovery/invariant failures are sanitized instead of exposing implementation diagnostics. Concrete wire codecs, listeners and authentication providers remain outside the protocol core.


## Hosted watch subscription boundary (P301)

Protocol watch subscriptions are scoped to one already-authorized `HostedSession`; predictable/local subscription identifiers are not global credentials and cannot resolve across sessions. Opening a watch still requires the runtime `Read + Watch` authority inherited from `SessionDatabase`. `NextWatch` consumes exact durable revision effects through the ordinary runtime watch; `CancelWatch`, `CloseWatch`, and `CloseSession` are wake/lifecycle operations only and have no mutation or writer-conflict authority.

Hosted sessions cap concurrently maintained watch subscriptions. One subscription has at most one in-flight consumer; competing consumers fail closed instead of serializing an unbounded waiter queue. A transport adapter is responsible for closing its `HostedSession` on peer/session termination so blocked watch work is cancelled deterministically. Concrete transport authentication, channel binding and anti-replay remain responsibilities of the future transport/auth composition, not of `SubscriptionId`.

## Hosted server composition boundary (P303)

`cfmd-host` is a trusted composition layer, not a network listener. Authentication providers may establish only a `PrincipalId`; authorization providers independently select runtime permissions. The host then constructs the restricted runtime session. A transport provider is therefore not implicitly trusted to mint `Write`, `HistoryRead`, `Watch`, or other grants merely because it accepted a socket/IPC peer.

No localhost, loopback address, process identity, database-path possession, or transport connection identifier is a trust signal by itself. Concrete transports must supply evidence to an explicitly configured authentication authority. Provider failures and authentication rejection are sanitized at the host boundary.

Active connections and per-connection in-flight frame execution are bounded. Closing a connection or the host closes the underlying hosted session and cancels blocked watch work. This lifecycle path does not mutate database Revision or acquire semantic conflict-resolution authority.

## Hosted security lifecycle (P304)

Session grants are live host-issued authority, not immutable copies embedded into each product value. `ReadContext`, `Plan`, `Candidate`, history entries and watch derivations share one session authority cell. Permission refresh changes that cell; revocation is terminal. Existing values therefore cannot preserve a removed `Write`/`Watch` grant. A commit already admitted past its final runtime authority check is not asynchronously rolled back by later revocation; subsequent authority transitions fail closed.

Authentication receives explicit channel-binding context. `Unbound` is a first-class state and carries no locality trust; a provider that requires replay-resistant binding must reject it. Expiring grants expose their deadline to the host. `cfmd-host` has no hidden timer/polling thread: transports/event loops can schedule the nearest deadline from `next_expiration()` and call `expire_due(now)`, which revokes/tears down expired sessions and wakes blocked watches. Graceful drain rejects new connections without revoking existing sessions; immediate close/revoke remains a separate operation.


## First-party local IPC security boundary (P305)

The local Unix transport does not trust localhost, Unix-socket reachability, endpoint path possession or same-machine execution. The socket is created with mode `0600` as defense in depth only. Every peer still presents explicit bounded authentication evidence, which is evaluated by the configured `Authenticator`; grants still come only from the configured `Authorizer`. P305 deliberately supplies `ChannelBinding::Unbound` rather than inventing cryptographic binding from locality metadata.

The authentication prelude has a fixed header and an evidence-length cap checked before evidence-body allocation. Request execution uses a bounded worker set capped by host in-flight limits and a bounded response queue. Socket EOF closes the hosted connection and therefore revokes/cancels session-scoped work, including blocked watch consumers. The transport never receives raw unrestricted database authority and cannot certify semantic writer compatibility.

### Single-file storage boundary

Single-file mode does not weaken the trust boundary by emitting hidden recovery sidecars. The published `*.cfmd` file is the only durability authority for the supported P308 path. Unsupported mutable authorities fail closed rather than redirecting to directory files. Encryption-at-rest remains a future orthogonal AEAD/key-management layer and is not treated as a substitute for host/session security.

### Single-file freshness / compaction integrity (P312)

External freshness is enforced over root-authoritative generation/WAL material rather than over a directory listing or the physical file length. Unpublished orphan bytes and obsolete compacted ranges therefore cannot satisfy an external freshness claim. A store carrying an external freshness binding must use the freshness-aware open path; the ordinary single-file open path fails closed.

Compaction is not an integrity reset. Relocated generation and WAL bytes are fully revalidated before the relocation root is published, and the relocation is required to preserve the same logical generation, Revision endpoint and freshness material. Copied-but-unpublished bytes have no authority. The obsolete tail is truncated only after durable root publication and live-journal reopen, so a crash cannot turn a partially copied compacted image into the acknowledged database state.

