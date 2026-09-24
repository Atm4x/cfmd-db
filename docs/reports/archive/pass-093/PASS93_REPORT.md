# PASS93 REPORT

**Status:** FINAL / REPLICATION MEMBERSHIP-QUORUM AUTHORITY ADVANCED

## Baseline and boundary

Baseline: frozen Pass92 (`cfmd_workspace_pass92_reic_branch_replication_authority.zip`). Source work started at **2026-09-22 23:46:14 UTC**. Production source was frozen at **23:57:58 UTC** after the final hostile/full-workspace gate. The freeze is intentionally earlier than the nominal 20-minute cutoff because the remaining #16 blocker is a new consensus-safety protocol, not a local patch. No `.rs` or Cargo manifest is modified after freeze.

The user-supplied `CFMD_RND_HIST21_22_2026-09-23_v4(1).zip` was inspected only to preserve ownership boundaries. It proposes a Universal Delta ABI / `ValidatedTransitionFrame` / compact-spill carrier program for #21/#22; Pass93 does **not** modify Group, TopK, maintained delta ABI, or their benchmark surface.

## Result

- Historical production closure remains **14 / 22**.
- Historical **#16 replication / consensus runtime — ADVANCED / PROD PARTIAL**.
- Historical #21/#22 remain external-R&D-owned and production untouched.
- Production source delta versus Pass92 is exactly:
  - `crates/kernel-durability/src/replication.rs`
  - `crates/kernel-durability/src/store.rs`
  - `crates/kernel-durability/src/lib.rs`
- Frozen source fingerprint: `3c115e497a83fa9bab20140044b5d969a6f4797bd3da37d2f500342492800978`.
- Final gate: fmt/check/strict Clippy PASS; **680 declared tests / 0 failed / 8 ignored**.

## #16 production advance

Pass92 already had independent durable branch journals, exact REIC causal cuts, origin namespaces, branch lifecycle, sequencer-slot uniqueness and monotone sequencer-epoch fencing. Pass93 adds the next authority layer without weakening the linear transaction WAL.

### Durable membership epochs

`ReplicationMembership` defines a non-empty member set and strict-majority threshold. Epoch zero is invalid. Bootstrap is explicit. Every later `ReplicationMembershipChange` must advance the epoch and carry the configured quorum of the previous membership. Membership frames are fsync-backed in the existing replication authority journal and reconstruct exactly on reopen.

### Explicit durability / visibility stages

`ReplicationEffectStage` separates:

1. `Received` — transient pre-durable transport state;
2. `LocalDurable` — effect frame is durably present in the local replication journal;
3. `QuorumDurable` — a durable structural quorum certificate exists for the **current membership epoch**;
4. `Published` — a separate durable publication frame exists.

Ingest never implies quorum. Quorum never implies publication. Publication never moves the store's single linear `durable_head`.

### Quorum certificates

`ReplicationQuorumCertificate` binds one exact `RevisionEffectId`, one membership epoch and a unique acknowledgement set. The store rejects non-members, insufficient quorum and stale membership epochs. Once a certificate is durable, a conflicting certificate for the same effect is rejected while exact replay is idempotent.

The acknowledgement identities are deliberately treated as already authenticated evidence. Pass93 does **not** claim that a bare `ReplicaId` is a cryptographic signature.

### Publication order and causal prefix

Publication requires:

- the effect itself is quorum durable;
- every replicated causal prerequisite is already quorum durable;
- publication is contiguous within the branch;
- the branch has not been retired.

Thus reader visibility cannot outrun the replicated causal quorum prefix. A branch may have a local-durable head ahead of its separately tracked published head.

## Hostile falsification

Pass93 tests:

- `LocalDurable` effect cannot publish before quorum;
- insufficient quorum is rejected without stage advancement;
- quorum + publication survive reopen while linear `durable_head` stays unchanged;
- membership change requires previous-epoch quorum;
- stale membership certificates are rejected after reconfiguration;
- non-member acknowledgement is rejected;
- later branch effect cannot publish before an earlier local-durable branch effect;
- an effect cannot publish while a remote causal predecessor lacks quorum durability;
- all Pass92 branch/restart/retirement/corruption/sequencer-fence tests remain green.

## Why #16 is not CLOSED

The remaining blocker is now precise rather than generic "networking".

A structural majority certificate cannot itself prevent two incompatible membership transitions on different replicas if an intersecting voter double-votes. Exhaustive 3-voter majority inspection shows every pair of distinct majorities intersects in one voter; without **durable vote-once state tied to an authenticated voter identity**, that shared voter can support both certificates. Therefore Pass93 does not equate threshold counting with consensus safety.

Still required for #16 closure:

- authenticated durable per-replica vote records / vote-once rules;
- election term / leader or equivalent decision-slot protocol;
- safe membership reconfiguration across replicas, not only local validation;
- quorum-loss and recovery semantics;
- real transport / anti-entropy / retry protocol;
- integration of certified-confluent coordination-free effects where appropriate.

## Next

Pass94 should continue #16 at the actual blocker: durable vote/election identity and consensus decision-slot semantics. Do not start network transport until the local consensus safety model can reject double voting and conflicting decisions deterministically. #21/#22 remain external-R&D-owned until that branch converges.
