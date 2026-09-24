# Pass117 frontier — Historic #17 authenticated durable store + external freshness / anti-rollback anchor

## Existing production substrate

1. `kernel-auth::SignedGenerationRecord` authenticates immutable generation components (manifest/checkpoint/metadata/optional prepared capsule) and chains `previous: AuthorityDigest`.
2. `SignedWalFrameRecord` authenticates WAL frames and `verify_wal_extension` enforces exact LSN/hash-chain continuity.
3. `compare_with_anchor` already detects store-id mismatch, rollback, same-generation fork, generation gaps, and wrong predecessor.
4. `FreshnessAnchor` already exposes the correct atomic external API: `read(store_id)` and `compare_and_advance(expected,next)`.
5. Pass112/#18 proves the local publication/GC protocol assuming filesystem durability axioms.
6. Pass115/#16 supplies authenticated peer evidence, quorum-loss fencing, transport and anti-entropy that can serve as one independent distributed freshness witness.

## Remaining #17 obligations

### 17A — anchor authority and durable encoding
- Define a canonical external anchor record binding `store_id`, generation authority digest, trust-root epoch and deployment-policy epoch.
- Add monotone compare-and-swap semantics; never permit blind overwrite.
- Authenticate anchor observations/responses so a local attacker cannot forge freshness.

### 17B — production store/recovery integration
- Before accepting recovered local generation, verify the signed generation record and compare it against the external anchor.
- Reject rollback/fork/gap before WAL replay or semantic publication.
- On a new locally durable generation, advance external anchor only after #18 publication prerequisites are durable; if anchor advance is uncertain, enter explicit authority-uncertain state rather than publish.
- Persist/restore trust-root and deployment-policy epochs under the same freshness cut.

### 17C — WAL freshness
- Anchor generation head plus final authenticated WAL-chain head/LSN where uncheckpointed durable WAL can be authoritative.
- Recovery must reject a validly signed but truncated/replayed older WAL suffix.

### 17D — real external backend
Preferred first production backends, without making any one part of the universal core:
- authenticated quorum anchor over the Pass115 replication transport, or
- application/provider supplied monotonic external CAS service implementing `FreshnessAnchor`.
A local file in the same rollback domain is explicitly insufficient.

### 17E — hostile matrix
- rollback local generation while external anchor is newer;
- fork same generation with a valid different signature;
- valid old trust/deployment policy snapshot;
- truncate/replay WAL after anchor advancement;
- crash before/after external compare-and-advance;
- anchor unavailable / split quorum;
- catch-up exactly one generation and multi-generation recovery protocol;
- independent-process anchor fault/restart tests.

## Closure rule
#17 closes only when at least one genuinely separate rollback domain is integrated and the main store recovery/publication path consumes it. The existing in-memory/test anchor abstraction alone is not sufficient.
