# IMPLEMENTATION REPORT — PASS94

## Production source delta

Exactly five source files differ from frozen Pass93:

1. `crates/kernel-durability/src/lib.rs`
2. `crates/kernel-durability/src/replication.rs`
3. `crates/kernel-durability/src/store.rs`
4. `crates/kernel-query/src/lib.rs`
5. `crates/kernel-query/src/delta_abi.rs` — new

## Durability / consensus changes

- added `ReplicationEffectVote` and `ReplicationMembershipVote` public authority-side values;
- added fsync-backed effect-vote and membership-vote frames to the existing replication journal;
- effect vote-once key: membership epoch + global ordered position + voter;
- membership vote-once key: previous membership epoch + voter;
- quorum certification now requires matching durable effect votes for every acknowledgement;
- non-bootstrap membership installation now requires same-term durable votes for the exact successor membership;
- replay reconstructs votes before dependent quorum/membership decisions;
- new restart hostiles reject conflicting effect votes across leaders and conflicting membership successors.

The implementation deliberately does not authenticate peers itself. The caller/security transport must authenticate voter identity before recording a vote; numeric `ReplicaId` alone is not trust evidence.

## V5 Delta ABI Stage 1

New `kernel-query::delta_abi` contains:

- `Weighted<R>`;
- `DeltaView<R>`;
- `DeltaSink<R>`;
- `CompactDelta<R>`;
- `InlineDelta<R, N>`;
- `AdaptiveDelta<R, N>`;
- `RelationDeltaView<'a>`.

`RelationDelta::as_delta_view()` exposes the compatibility view without allocating or cloning rows. `AdaptiveDelta` starts in fixed array-backed inline storage, spills after capacity is exhausted, and retains spill allocation across `clear()` for reuse.

No existing maintained execution function was redirected to this ABI in Pass94.

## Verification

- `cargo fmt --all -- --check` — PASS
- `cargo check --workspace --all-targets` — PASS
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS
- `cargo test --workspace --all-targets` — PASS
- declared tests: **685**
- failures: **0**
- ignored: **8**

Frozen source fingerprint: `b28009479bcbfa8eb84ece69108f05c680d0f82d855b310d9e69d1dfd9386da8`.
