# PASS113 frontier — historical #16 replication/consensus runtime

Historical #18 is mechanically closed in Pass112. The next active frontier is #16.

## 16A — term / election / locking authority

Implement first-class durable consensus authority rather than accepting `DurableSequencerOrder` as an external witness:

1. durable current/promised term;
2. candidate/leader identity and vote-for-leader once per term;
3. stale-term rejection/fencing;
4. per-decision-position accepted/locked value;
5. safe carry-forward rule for a later-term leader;
6. membership-epoch compatibility including transition/joint quorum cases;
7. restart replay reconstructs term, vote and lock authority exactly;
8. hostile tests for double vote, stale leader, conflicting lock, restart and membership transition.

## 16B onward

After 16A correctness, add authenticated peer evidence, quorum-loss/recovery, transport/anti-entropy and distributed multi-process fault assurance. Do not claim #16 closed before those layers exist.
