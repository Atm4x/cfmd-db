# IMPLEMENTATION REPORT — Pass115

## Scope

Close Historical #16 after the Pass114 16B closure candidate and implement 16C:
authenticated transport, anti-entropy, failure detection, and distributed multi-process
fault assurance.

## Pass114 gate completion

Pass114 source was first tested unchanged with the previously missing full workspace gate.
Result: PASS. This promotes 16B to COMPLETE before any Pass115 feature work.

## Production implementation

### `replication_transport.rs`

Added a transport-neutral application protocol rather than hard-coding TCP/QUIC:

- `ReplicationTransportFrame` / `SignedReplicationTransportFrame`;
- stable `CFTR` wire format and version;
- domain-separated Ed25519 transport signing message;
- cluster, trust epoch, sender/key and monotone session sequence checks;
- nested peer-evidence sender/key/trust binding;
- `ReplicationTransportIngress` with authenticated replay/dedup boundary;
- `accept_and_observe` so unauthenticated traffic cannot refresh failure detection.

### Anti-entropy

Added bounded non-authoritative lock-frontier exchange:

- `ReplicationAntiEntropySummary`;
- digest over ordered `ReplicationLockSummary` values;
- `ReplicationAntiEntropyRequest`;
- bounded `ReplicationAntiEntropyChunk` (`MAX_ANTI_ENTROPY_LOCKS = 4096`);
- explicit `InSync / ExchangeRequired / MembershipMismatch` relation;
- `DurableRevisionStore` helpers to expose current summary and bounded chunks.

Anti-entropy data never installs decision authority by itself. Missing/conflicting locks must
still be reconciled through the authenticated decision/recovery protocol from 16A/16B.

### Failure detector

Added logical-clock `ReplicationFailureDetector`:

- only authenticated observations refresh peer liveness;
- monotone logical time;
- reachability/quorum calculation against current membership;
- emits only `ReplicationQuorumLoss` observations;
- store helper converts loss into the existing durable safety fence;
- no automatic recovery path exists.

Therefore false suspicion can hurt liveness but cannot create consensus authority.

### Store transport routing

`DurableRevisionStore::durably_accept_replication_transport_frame` now bridges transport to
the existing durable consensus journal:

- peer-evidence payloads are verified by transport and then durably verified/admitted by 16B;
- heartbeat and anti-entropy payloads remain advisory;
- stale membership transport payloads are rejected.

## Falsification

Added unit/hostile coverage for:

- canonical wire roundtrip;
- signature tamper;
- session replay/non-monotone sequence;
- anti-entropy mismatch detection;
- bounded/ordered chunks;
- advisory failure detector;
- durable quorum-loss fence integration;
- authority-bearing transport routing vs advisory heartbeat.

Added `tests/replication_transport_multiprocess.rs`, which launches separate OS processes and
checks:

- valid signed frame delivery;
- duplicate replay rejection in one receiver session;
- signature tamper rejection;
- torn/truncated sender process (`exit 17`) rejection by the receiver.

## Gates

- `cargo fmt --all -- --check`: PASS
- `cargo check --workspace --all-targets --offline`: PASS
- `cargo clippy --workspace --all-targets --offline -- -D warnings`: PASS
- `kernel-durability`: 91 unit tests PASS
- multiprocess transport integration: 2 tests PASS
- full workspace: 738 declared / 730 passed / 0 failed / 8 ignored

## Result

Historical #16 is PROD CLOSED in Pass115.
