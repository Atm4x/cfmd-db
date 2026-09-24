# PASS114 frontier — historical #16B

16A is COMPLETE in Pass113. #16 remains PROD PARTIAL.

## 16B.1 — authenticated peer evidence

1. Define a stable canonical signed payload/domain separator for term promises, leader votes, decision votes/locks and joint membership transition evidence.
2. Bind evidence to cluster identity, membership epoch, term, voter identity and exact payload digest to prevent replay/cross-cluster substitution.
3. Verify evidence before it reaches `ReplicationAuthorityJournal`; the journal must consume verified evidence, not caller assertions.
4. Model key epoch/rotation and make stale/revoked peer keys fail closed. Keep root-of-trust ownership dependency explicit with #10/#17.
5. Durable replay must preserve proof identity/digest and never silently downgrade authenticated records to structural acknowledgements.

## 16B.2 — quorum loss / recovery

1. First-class authority state for `QuorumAvailable`, `QuorumLost`, recovery/rejoin.
2. On quorum loss: no new leader certificate, no new decision lock, no membership transition/publication requiring consensus authority.
3. Define exact reader behavior and non-authoritative local durability allowed while quorum is absent.
4. Recovery must require a safe term advance and reconcile highest durable locks before leadership can resume.
5. Hostile restart tests for partial peer evidence, stale leader after partition, competing recovery attempts and membership transition during quorum loss.

## 16C+

Transport/anti-entropy, failure detection and distributed multi-process fault assurance remain after 16B. Do not close #16 before these exist.
