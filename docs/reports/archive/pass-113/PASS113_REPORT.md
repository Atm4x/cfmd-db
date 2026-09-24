# PASS113 — historical #16A term/election/locking authority

Freeze: 2026-09-23T15:38:34Z
Production Rust fingerprint: 623baba04385b4655738b537382181927a5a24b2e7dfeb50ad2e4dccef245809

## Result

Historical #16 remains **ADVANCED / PROD PARTIAL**, but **16A term/election/locking authority is COMPLETE in Pass113**.

Pass113 replaces the previous assumption that `DurableSequencerOrder` is the only ordering/authority witness once consensus mode is activated. The replication journal now has first-class durable consensus state while retaining backward-compatible legacy admission for stores that have not activated a leader certificate.

## Implemented durable consensus authority

1. **Durable term promises**
   - `ReplicationTermPromise { voter, membership_epoch, term }`.
   - Promise regression is rejected.
   - Replay reconstructs the exact highest promised term per voter and membership epoch.

2. **Vote-for-leader once per term**
   - `ReplicationLeaderVote` is durable.
   - One voter cannot support two candidates in one membership epoch/term.
   - A higher durable promise fences later attempts to vote in a stale term.

3. **Quorum leader certificate**
   - `ReplicationLeaderCertificate` requires the configured membership quorum and exact durable leader-vote evidence.
   - A certificate below an observed higher promised term is rejected.

4. **Per-position accepted value and durable lock**
   - `ReplicationDecisionVote` is term/leader/position/effect bound.
   - `ReplicationDecisionLock` requires quorum-matching durable decision votes.
   - A consensus-activated ordinary replication quorum certificate cannot bypass the decision lock.

5. **Safe later-term carry-forward**
   - Once position P is locked to effect E in term T, any later-term decision for P must use E and explicitly name `carried_from_term = T`.
   - Conflicting effect replacement is rejected even under a new leader.
   - A later lock must strictly advance the term and carry the prior lock term.

6. **Membership-epoch compatibility / joint quorum**
   - `ReplicationJointMembershipCertificate` binds one successor configuration to:
     - a certified leader term in the old membership;
     - a quorum of the old membership;
     - a quorum of the successor membership.
   - This supports even fully disjoint old/new membership sets without inventing a membership-overlap restriction.
   - A later promised term fences a previously created joint certificate at install time.

7. **Restart/replay**
   - Journal frame kinds were appended without changing old frame payload formats.
   - Term promises, leader votes/certificates, decision votes/locks and joint membership certificates replay into the same authority state.
   - Incomplete-tail/checksum journal behavior remains covered by the existing replication tests.

## Compatibility boundary

Existing Pass94 membership/effect-vote/quorum APIs remain available. If no leader certificate exists for the current membership epoch, the old durable admission behavior remains valid. Once a first-class leader certificate exists for that epoch, effect quorum certification additionally requires the matching consensus `DecisionLock`; this prevents a legacy quorum path from bypassing the new authority.

## Hostile tests added

- same voter / same term / conflicting leader rejected;
- term promise regression rejected;
- stale leader vote rejected after higher promise;
- term + leader authority survives restart;
- conflicting value at an already locked position rejected;
- later-term same-value decision without explicit carry-forward rejected;
- later-term same-value carry-forward succeeds and survives restart;
- effect quorum cannot bypass consensus lock after leader activation;
- joint membership change is rejected without successor quorum evidence;
- fully disjoint old/new configurations transition with explicit old+new quorum certificate;
- joint membership install is fenced by a later promised term.

## Gates

- `cargo fmt --all -- --check` — PASS.
- `cargo check --workspace --all-targets` — PASS.
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS.
- replication slice — PASS.
- full workspace — **715 declared / 707 passed / 0 failed / 8 ignored**.

## Historical status

Overall ledger stays **17/22 PROD CLOSED**. #16 is not closed because 16B+ still require authenticated peer evidence, explicit quorum-loss/recovery behavior, transport/anti-entropy and distributed multi-process fault assurance.
