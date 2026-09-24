# PASS92 REPORT

**Status:** FINAL / DURABLE REIC BRANCH AUTHORITY + ORDERED REPLICATION ADMISSION

## Baseline and wall-clock
Baseline: frozen Pass91 (`cfmd_workspace_pass91_semantic_deployment_reic_boundary.zip`). Source integration started at **2026-09-22 23:19:14 UTC** and was frozen early at **23:36:43 UTC**, before the nominal 20-minute boundary, after the complete workspace gate passed. No production source was changed after freeze.

## Production result
- Historical **#8 durable causal effect ledger / REIC DAG lifecycle — PROD CLOSED**.
- Historical **#16 replication / consensus runtime — PROD PARTIAL / materially advanced**.
- Historical production closure is now **14 / 22**.
- Historical #21/#22 remain **DEFERRED / external R&D owner**; their maintained Group/TopK production source was not touched.
- Production source delta versus Pass91 is exactly:
  - `crates/kernel-durability/src/lib.rs`
  - `crates/kernel-durability/src/metadata.rs`
  - `crates/kernel-durability/src/replication.rs` (new)
  - `crates/kernel-durability/src/store.rs`
- Frozen source fingerprint: `f028bb463c266a1de4bde3d8c094d9488f17e9de6fe19c882cec876dd673ae9b`.
- Final gate: fmt/check/strict Clippy/full workspace tests PASS; **676 declared / 0 failed / 8 ignored**.

## #8 closure — durable non-head REIC lifecycle
Pass92 adds a dedicated append-only replication authority journal without weakening the linear transaction WAL invariant. A remote/non-head branch effect is still the existing `DurableRevisionEffectRecord` carrying the existing `DurableTransactionIntent`; there is no second mutation ontology.

Each replicated effect now has:
- globally disjoint effect identity `ReplicaId << 64 | origin_sequence`; local effects are permanently fenced to origin namespace zero;
- a stable `ReplicationBranchId` whose head may advance independently of the single published `durable_head`;
- exact causal prerequisites checked against the union of local and replicated revision frontiers;
- existing semantic implementation authorization/install checks before admission;
- an append+`sync_data` durability boundary in `replication.cfre`;
- replay-time checksum, branch-head, causal-cut and semantic-availability validation;
- durable retirement that survives restart and forbids branch resurrection;
- reconstruction of a down-closed branch `RevisionEffectIdeal` across local and replicated ancestors.

This closes the missing lifecycle from Pass91: the causal ledger is no longer limited to one mutable publication head, while reader publication remains single-head and separate.

## #16 advance — conservative ordered admission, not fake consensus
Current durable effects are still `OpaqueNonConfluent`, so Pass92 only admits them with an explicit `DurableSequencerOrder`. The journal enforces:
- uniqueness of `(sequencer, epoch, position)` slots;
- persistent monotone sequencer-epoch fencing;
- rejection of stale epochs;
- idempotent duplicate effect delivery;
- exact origin/sequence identity binding;
- down-closure before durable admission.

This is a real production admission/fencing boundary, but #16 is intentionally not CLOSED. Pass92 does **not** implement membership changes, quorum voting/certificates, leader election, quorum-loss behavior, networking/anti-entropy transport, or certified-confluent coordination-free admission. The R&D reference explicitly leaves those systems pieces open.

## Hostile falsification
Pass92 tests prove:
- two independent branches can fork from the same local root without moving the published head;
- one branch can advance while another remains independent;
- restart reconstructs both branch heads and their REIC ideals;
- retirement survives restart and blocks later branch advance;
- a stale/wrong causal cut is rejected before journal growth;
- one sequencer slot cannot bind two effect IDs;
- a newer sequencer epoch permanently fences the older epoch;
- exact duplicate delivery is idempotent and does not append another frame;
- an incomplete final journal frame is safely truncated on reopen;
- checksum corruption of a complete durable frame is fail-closed and is never silently truncated.

## Next
Pass93 should continue **#16** with the next authority layer: durable membership epochs + quorum certificate semantics + explicit transitions between Received / LocalDurable / QuorumDurable / Published. Networking can remain a transport adapter, but quorum authority and failure semantics must be production-defined before #16 can close. #21/#22 stay untouched until their external R&D converges.
